use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use strsim::jaro_winkler;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame,
};
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

use crate::models::{ChainLink, Session, Status, TuiEvent};
use crate::store::Store;

/// Unified item for Memory tab - can be a memory key or a chain name
#[derive(Clone)]
enum MemoryItem {
    Memory(String),      // memory key
    Chain(String),       // chain name
}

impl MemoryItem {
    fn name(&self) -> &str {
        match self {
            MemoryItem::Memory(k) => k,
            MemoryItem::Chain(n) => n,
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq)]
enum Tab {
    #[default]
    Active,
    Pending,
    Memory,
    History,
}

pub struct App {
    store: Store,
    rx: Receiver<TuiEvent>,
    tab: Tab,
    sessions: Vec<Session>,
    active_tools: std::collections::HashMap<String, String>, // session_id -> tool_name
    last_msgs: std::collections::HashMap<String, String>, // session_id -> last message preview
    last_hook_details: std::collections::HashMap<String, String>, // session_id -> full hook detail (first 1k chars)
    session_state: ListState,
    pending: Vec<(String, String)>,
    pending_state: ListState,
    history: Vec<Session>,
    // Memory tab (unified: memories + chains)
    memory_keys: Vec<String>,
    chain_names: Vec<String>,
    memory_filtered: Vec<(MemoryItem, f64)>, // (item, score)
    memory_state: ListState,
    memory_search: String,
    memory_content: Option<String>,   // selected memory content OR chain segments
    content_scroll: u16,              // scroll offset for content panel
    // Input
    input_mode: bool,
    input: String,
    search_mode: bool, // memory search mode
}

impl App {
    pub fn new(store: Store, rx: Receiver<TuiEvent>) -> Self {
        Self {
            store,
            rx,
            tab: Tab::Active,
            sessions: vec![],
            active_tools: std::collections::HashMap::new(),
            last_msgs: std::collections::HashMap::new(),
            last_hook_details: std::collections::HashMap::new(),
            session_state: ListState::default(),
            pending: vec![],
            pending_state: ListState::default(),
            history: vec![],
            memory_keys: vec![],
            chain_names: vec![],
            memory_filtered: vec![],
            memory_state: ListState::default(),
            memory_search: String::new(),
            memory_content: None,
            content_scroll: 0,
            input_mode: false,
            input: String::new(),
            search_mode: false,
        }
    }

    pub async fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        self.refresh().await?;
        loop {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press && self.handle_key(key.code).await? {
                        break;
                    }
                }
            }
            while let Ok(ev) = self.rx.try_recv() {
                match ev {
                    TuiEvent::Refresh | TuiEvent::NewSession | TuiEvent::NewQuestion | TuiEvent::SessionDone => {
                        self.refresh().await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn refresh(&mut self) -> Result<()> {
        let ids = self.store.list_active().await?;
        self.sessions = futures::future::join_all(ids.iter().map(|id| self.store.get_session(id)))
            .await
            .into_iter()
            .filter_map(|r| r.ok().flatten())
            .collect();
        // Fetch active tools for each session
        self.active_tools.clear();
        for s in &self.sessions {
            if let Ok(Some(tool)) = self.store.get_active_tool(&s.id).await {
                self.active_tools.insert(s.id.clone(), tool);
            }
        }
        self.pending = self.store.list_waiting().await?;
        let hist_ids = self.store.list_history(20).await?;
        self.history = futures::future::join_all(hist_ids.iter().map(|id| self.store.get_session(id)))
            .await
            .into_iter()
            .filter_map(|r| r.ok().flatten())
            .collect();
        // Fetch last hook for all sessions (shows last activity with details)
        self.last_msgs.clear();
        self.last_hook_details.clear();
        let all_sessions: Vec<&Session> = self.sessions.iter().chain(self.history.iter()).collect();
        for s in all_sessions {
            if let Ok(hooks) = self.store.get_hooks(&s.id, 1).await {
                if let Some(hook) = hooks.last() {
                    let kind = if hook.kind == "pre" { "→" } else { "✓" };
                    // Extract useful preview from tool input (priority fields for common tools)
                    let meta_str = if let Some(obj) = hook.meta.as_object() {
                        // Priority order: file_path, command, pattern, query, url, skill, prompt
                        let priority_keys = ["file_path", "command", "pattern", "query", "url", "skill", "prompt"];
                        let mut found = None;
                        for key in priority_keys {
                            if let Some(serde_json::Value::String(val)) = obj.get(key) {
                                let val = val.replace('\n', " ");
                                found = Some(if val.len() > 45 { format!("{}...", &val[..42]) } else { val });
                                break;
                            }
                        }
                        found.unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let preview = if meta_str.is_empty() {
                        format!("{} {}", kind, hook.task)
                    } else {
                        format!("{} {} ({})", kind, hook.task, meta_str)
                    };
                    self.last_msgs.insert(s.id.clone(), preview);

                    // Store full hook details for detail panel (first 1000 chars)
                    let full_meta = serde_json::to_string_pretty(&hook.meta).unwrap_or_default();
                    let detail = format!("Last: {} {}\n\n{}", kind, hook.task,
                        if full_meta.len() > 1000 { format!("{}...", &full_meta[..1000]) } else { full_meta });
                    self.last_hook_details.insert(s.id.clone(), detail);
                }
            }
        }
        // Load memory keys and chain names
        self.memory_keys = self.store.list_memory_keys().await.unwrap_or_default();
        self.chain_names = self.store.list_chain_names().await.unwrap_or_default();
        self.filter_items();
        Ok(())
    }

    fn filter_items(&mut self) {
        if self.memory_search.is_empty() {
            // Interleave chains and memories: chains first (marked with 🔗), then memories
            let mut items: Vec<(MemoryItem, f64)> = Vec::new();
            for name in &self.chain_names {
                items.push((MemoryItem::Chain(name.clone()), 1.0));
            }
            for key in &self.memory_keys {
                items.push((MemoryItem::Memory(key.clone()), 1.0));
            }
            self.memory_filtered = items;
        } else {
            let query = self.memory_search.to_lowercase();
            // Score both chains and memories together
            let mut scored: Vec<(MemoryItem, f64)> = Vec::new();

            for name in &self.chain_names {
                let n_lower = name.to_lowercase();
                let base = jaro_winkler(&n_lower, &query);
                let boost = if n_lower.contains(&query) { 0.3 } else { 0.0 };
                let score = (base + boost).min(1.0);
                if score > 0.4 {
                    scored.push((MemoryItem::Chain(name.clone()), score));
                }
            }
            for key in &self.memory_keys {
                let k_lower = key.to_lowercase();
                let base = jaro_winkler(&k_lower, &query);
                let boost = if k_lower.contains(&query) { 0.3 } else { 0.0 };
                let score = (base + boost).min(1.0);
                if score > 0.4 {
                    scored.push((MemoryItem::Memory(key.clone()), score));
                }
            }
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            self.memory_filtered = scored;
        }
        // Reset selection if out of bounds
        if self.memory_state.selected().map_or(false, |i| i >= self.memory_filtered.len()) {
            self.memory_state.select(if self.memory_filtered.is_empty() { None } else { Some(0) });
        }
    }

    async fn load_selected_item(&mut self) {
        self.content_scroll = 0; // Reset scroll on new selection
        if let Some(i) = self.memory_state.selected() {
            if let Some((item, _)) = self.memory_filtered.get(i) {
                match item {
                    MemoryItem::Memory(key) => {
                        if let Ok(Some(mem)) = self.store.get_memory(key).await {
                            self.memory_content = Some(format!(
                                "📝 Memory: {}\nKind: {}\nSession: {}\n\n{}",
                                mem.key, mem.kind, mem.session_id, mem.content
                            ));
                            return;
                        }
                    }
                    MemoryItem::Chain(name) => {
                        if let Ok(links) = self.store.get_chain_links(name).await {
                            let formatted = self.format_chain_links(name, &links);
                            self.memory_content = Some(formatted);
                            return;
                        }
                    }
                }
            }
        }
        self.memory_content = None;
    }

    fn format_chain_links(&self, chain_name: &str, links: &[ChainLink]) -> String {
        if links.is_empty() {
            return format!("🔗 Chain: {}\n\n(no links yet)", chain_name);
        }
        let mut output = format!("🔗 Chain: {} ({} links)\n", chain_name, links.len());
        output.push_str("─".repeat(40).as_str());
        output.push('\n');

        for (i, link) in links.iter().enumerate() {
            let ts = chrono::DateTime::from_timestamp(link.ts, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| link.ts.to_string());
            output.push_str(&format!("\n[{}] {} ({})\n", i + 1, link.slug, ts));
            output.push_str(&format!("Session: {}\n", link.session_id));
            // Show first 500 chars of content
            let preview = if link.content.len() > 500 {
                format!("{}...", &link.content[..500])
            } else {
                link.content.clone()
            };
            output.push_str(&format!("\n{}\n", preview));
            output.push_str("─".repeat(40).as_str());
            output.push('\n');
        }
        output
    }

    async fn handle_key(&mut self, code: KeyCode) -> Result<bool> {
        if self.search_mode {
            match code {
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.memory_search.clear();
                    self.filter_items();
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                    self.load_selected_item().await;
                }
                KeyCode::Backspace => {
                    self.memory_search.pop();
                    self.filter_items();
                }
                KeyCode::Char(c) => {
                    self.memory_search.push(c);
                    self.filter_items();
                }
                _ => {}
            }
            return Ok(false);
        }
        if self.input_mode {
            match code {
                KeyCode::Esc => self.input_mode = false,
                KeyCode::Enter => {
                    self.submit().await?;
                    self.input_mode = false;
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(c) => self.input.push(c),
                _ => {}
            }
        } else {
            match code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Tab => {
                    self.tab = match self.tab {
                        Tab::Active => Tab::Pending,
                        Tab::Pending => Tab::Memory,
                        Tab::Memory => Tab::History,
                        Tab::History => Tab::Active,
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.next();
                    if self.tab == Tab::Memory { self.load_selected_item().await; }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.prev();
                    if self.tab == Tab::Memory { self.load_selected_item().await; }
                }
                KeyCode::Char('y') => {
                    self.input = "yes".into();
                    self.submit().await?;
                }
                KeyCode::Char('n') => {
                    self.input = "no".into();
                    self.submit().await?;
                }
                KeyCode::Char('e') | KeyCode::Enter => {
                    if self.tab == Tab::Memory {
                        self.load_selected_item().await;
                    } else {
                        self.input_mode = true;
                    }
                }
                KeyCode::Char('/') if self.tab == Tab::Memory => {
                    self.search_mode = true;
                    self.memory_search.clear();
                }
                KeyCode::Char('r') => self.refresh().await?,
                KeyCode::Char('d') => {
                    if self.tab == Tab::Memory {
                        self.delete_selected_item().await?;
                    } else {
                        self.archive_selected().await?;
                    }
                }
                // Scroll content panel (Memory tab)
                KeyCode::Char('l') | KeyCode::PageDown if self.tab == Tab::Memory => {
                    self.content_scroll = self.content_scroll.saturating_add(5);
                }
                KeyCode::Char('h') | KeyCode::PageUp if self.tab == Tab::Memory => {
                    self.content_scroll = self.content_scroll.saturating_sub(5);
                }
                _ => {}
            }
        }
        Ok(false)
    }

    async fn delete_selected_item(&mut self) -> Result<()> {
        if let Some(i) = self.memory_state.selected() {
            if let Some((item, _)) = self.memory_filtered.get(i).cloned() {
                match item {
                    MemoryItem::Memory(key) => {
                        self.store.delete_memory(&key).await?;
                        self.refresh().await?;
                    }
                    MemoryItem::Chain(name) => {
                        self.store.delete_chain(&name).await?;
                        self.refresh().await?;
                    }
                }
            }
        }
        Ok(())
    }

    fn next(&mut self) {
        match self.tab {
            Tab::Active => {
                let i = self
                    .session_state
                    .selected()
                    .map(|i| (i + 1).min(self.sessions.len().saturating_sub(1)))
                    .unwrap_or(0);
                self.session_state.select(Some(i));
            }
            Tab::Pending => {
                let i = self
                    .pending_state
                    .selected()
                    .map(|i| (i + 1).min(self.pending.len().saturating_sub(1)))
                    .unwrap_or(0);
                self.pending_state.select(Some(i));
            }
            Tab::Memory => {
                let i = self
                    .memory_state
                    .selected()
                    .map(|i| (i + 1).min(self.memory_filtered.len().saturating_sub(1)))
                    .unwrap_or(0);
                self.memory_state.select(Some(i));
            }
            Tab::History => {}
        }
    }

    fn prev(&mut self) {
        match self.tab {
            Tab::Active => {
                let i = self
                    .session_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.session_state.select(Some(i));
            }
            Tab::Pending => {
                let i = self
                    .pending_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.pending_state.select(Some(i));
            }
            Tab::Memory => {
                let i = self
                    .memory_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.memory_state.select(Some(i));
            }
            Tab::History => {}
        }
    }

    async fn submit(&mut self) -> Result<()> {
        let session_id = match self.tab {
            Tab::Pending => {
                self.pending_state.selected()
                    .and_then(|i| self.pending.get(i))
                    .map(|(id, _)| id.clone())
            }
            Tab::Active => {
                // Allow answering from Active tab if selected session has pending question
                self.session_state.selected()
                    .and_then(|i| self.sessions.get(i))
                    .filter(|s| matches!(s.status, Status::Waiting { .. }))
                    .map(|s| s.id.clone())
            }
            _ => None,
        };

        if let Some(id) = session_id {
            let answer = if self.input.is_empty() { "yes".into() } else { self.input.clone() };
            self.store.set_answer(&id, &answer).await?;
            self.input.clear();
            self.refresh().await?;
        }
        Ok(())
    }

    async fn archive_selected(&mut self) -> Result<()> {
        let session_id = match self.tab {
            Tab::Active => {
                self.session_state.selected()
                    .and_then(|i| self.sessions.get(i))
                    .map(|s| s.id.clone())
            }
            Tab::Pending => {
                self.pending_state.selected()
                    .and_then(|i| self.pending.get(i))
                    .map(|(id, _)| id.clone())
            }
            Tab::Memory | Tab::History => None,
        };

        if let Some(id) = session_id {
            self.store.mark_done(&id).await?;
            self.refresh().await?;
        }
        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(f.area());

        let pending_title = format!("Pending ({})", self.pending.len());
        let total_items = self.chain_names.len() + self.memory_keys.len();
        let memory_title = format!("Memory ({})", total_items);
        let titles: Vec<&str> = vec!["Active", &pending_title, &memory_title, "History"];
        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title(" tinymem "))
            .select(match self.tab {
                Tab::Active => 0,
                Tab::Pending => 1,
                Tab::Memory => 2,
                Tab::History => 3,
            })
            .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        f.render_widget(tabs, chunks[0]);

        match self.tab {
            Tab::Active => self.draw_active(f, chunks[1]),
            Tab::Pending => self.draw_pending(f, chunks[1]),
            Tab::Memory => self.draw_memory(f, chunks[1]),
            Tab::History => self.draw_history(f, chunks[1]),
        }
        self.draw_status(f, chunks[2]);
    }

    fn draw_active(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|s| {
                let has_active_tool = self.active_tools.contains_key(&s.id);
                let (icon, color) = match &s.status {
                    Status::Waiting { .. } => ("?", Color::Yellow),
                    Status::Done => ("○", Color::Gray),
                    Status::Active if has_active_tool => ("⚙", Color::Cyan),
                    Status::Active => ("●", Color::Green),
                };
                let name = s.name.as_deref().unwrap_or(&s.id);
                let last_msg = self.last_msgs.get(&s.id).map(|m| m.as_str()).unwrap_or("");
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{icon} "), Style::default().fg(color)),
                        Span::raw(name),
                    ]),
                    Line::from(Span::styled(last_msg, Style::default().dim())),
                ])
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Sessions "))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, chunks[0], &mut self.session_state);

        if let Some(i) = self.session_state.selected() {
            if let Some(s) = self.sessions.get(i) {
                let active_tool = self.active_tools.get(&s.id);
                let (status_str, hint) = match (&s.status, active_tool) {
                    (Status::Waiting { question, .. }, _) => (
                        format!("⚡ QUESTION:\n{question}"),
                        "\n\n→ Press [y]es [n]o or [e]dit to answer".to_string()
                    ),
                    (Status::Active, Some(tool)) => (
                        format!("🔧 RUNNING: {tool}"),
                        "\n\n⏳ Tool in progress".to_string()
                    ),
                    (Status::Active, None) => ("Active".into(), String::new()),
                    (Status::Done, _) => ("Done".into(), String::new()),
                };
                let hook_detail = self.last_hook_details.get(&s.id)
                    .map(|d| format!("\n\n{}", d))
                    .unwrap_or_default();
                let detail = format!(
                    "Agent: {}\nCWD: {}\nID: {}\n\n{}{}{}",
                    s.agent, s.cwd, s.id, status_str, hint, hook_detail
                );
                let p = Paragraph::new(detail)
                    .block(Block::default().borders(Borders::ALL).title(" Detail "))
                    .wrap(Wrap { trim: true });
                f.render_widget(p, chunks[1]);
            }
        }
    }

    fn draw_pending(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        let items: Vec<ListItem> = self
            .pending
            .iter()
            .map(|(id, q)| {
                ListItem::new(vec![
                    Line::from(id.clone()).bold(),
                    Line::from(q.clone()).dim(),
                ])
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Pending Questions "),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, chunks[0], &mut self.pending_state);

        if let Some(i) = self.pending_state.selected() {
            if let Some((id, q)) = self.pending.get(i) {
                let detail = format!(
                    "Session: {id}\n\nQuestion:\n{q}\n\nType response or press [y]es / [n]o"
                );
                let p = Paragraph::new(detail)
                    .block(Block::default().borders(Borders::ALL).title(" Answer "))
                    .wrap(Wrap { trim: true });
                f.render_widget(p, chunks[1]);
            }
        }
    }

    fn draw_memory(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        let counts = format!("🔗{} 📝{}", self.chain_names.len(), self.memory_keys.len());
        let title = if self.search_mode {
            format!(" [/{}] {} ", self.memory_search, counts)
        } else if !self.memory_search.is_empty() {
            format!(" (filter: {}) {} ", self.memory_search, counts)
        } else {
            format!(" [/] search {} ", counts)
        };

        let items: Vec<ListItem> = self
            .memory_filtered
            .iter()
            .map(|(item, score)| {
                let score_str = if *score < 1.0 { format!(" ({:.0}%)", score * 100.0) } else { String::new() };
                let (icon, color) = match item {
                    MemoryItem::Chain(_) => ("🔗", Color::Cyan),
                    MemoryItem::Memory(_) => ("📝", Color::Green),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", icon), Style::default().fg(color)),
                    Span::raw(item.name()),
                    Span::styled(score_str, Style::default().dim()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, chunks[0], &mut self.memory_state);

        let content = self.memory_content.as_deref().unwrap_or("Select a chain or memory to view");
        let scroll_info = if self.content_scroll > 0 { format!(" Content [↑{}] ", self.content_scroll) } else { " Content [h/l] ".into() };
        let p = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title(scroll_info))
            .wrap(Wrap { trim: false })
            .scroll((self.content_scroll, 0));
        f.render_widget(p, chunks[1]);
    }

    fn draw_history(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .history
            .iter()
            .map(|s| {
                let name = s.name.as_deref().unwrap_or(&s.id);
                let last_msg = self.last_msgs.get(&s.id).map(|m| m.as_str()).unwrap_or("");
                ListItem::new(vec![
                    Line::from(format!("○ {name}")),
                    Line::from(Span::styled(last_msg, Style::default().dim())),
                ])
            })
            .collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" History (recent 20) "),
        );
        f.render_widget(list, area);
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let help = if self.search_mode {
            format!(" Search: {}_ | [Enter] select | [Esc] clear ", self.memory_search)
        } else if self.input_mode {
            format!(" Input: {}_ | [Enter] submit | [Esc] cancel ", self.input)
        } else if self.tab == Tab::Memory {
            " [/] search | [j/k] navigate | [d]elete | [r]efresh | [q]uit ".into()
        } else {
            " [Tab] switch | [j/k] navigate | [y]es [n]o [e]dit | [d]one | [r]efresh | [q]uit ".into()
        };
        let style = if self.search_mode || self.input_mode {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().dim()
        };
        let p = Paragraph::new(help)
            .style(style)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(p, area);
    }
}
