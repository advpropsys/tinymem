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

use crate::models::{Artifact, ChainLink, Session, Status, TuiEvent};
use crate::store::Store;

#[derive(Default, Clone, Copy, PartialEq)]
enum Tab {
    #[default]
    Active,
    Chains,
    Artifacts,
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
    history: Vec<Session>,
    // Chains tab
    chains: Vec<(String, usize)>,     // (chain_name, link_count)
    chains_filtered: Vec<(String, usize, f64)>, // (name, count, score)
    chain_state: ListState,
    chain_search: String,
    chain_content: Option<String>,
    chain_scroll: u16,
    // Artifacts tab
    artifacts: Vec<Artifact>,
    artifacts_filtered: Vec<(Artifact, f64)>, // (artifact, score)
    artifact_state: ListState,
    artifact_search: String,
    artifact_content: Option<String>,
    artifact_scroll: u16,
    // Input
    input_mode: bool,
    input: String,
    search_mode: bool,
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
            history: vec![],
            chains: vec![],
            chains_filtered: vec![],
            chain_state: ListState::default(),
            chain_search: String::new(),
            chain_content: None,
            chain_scroll: 0,
            artifacts: vec![],
            artifacts_filtered: vec![],
            artifact_state: ListState::default(),
            artifact_search: String::new(),
            artifact_content: None,
            artifact_scroll: 0,
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
                    TuiEvent::Refresh | TuiEvent::NewSession | TuiEvent::SessionDone => {
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
                    let meta_str = if let Some(obj) = hook.meta.as_object() {
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
                    let full_meta = serde_json::to_string_pretty(&hook.meta).unwrap_or_default();
                    let detail = format!("Last: {} {}\n\n{}", kind, hook.task,
                        if full_meta.len() > 1000 { format!("{}...", &full_meta[..1000]) } else { full_meta });
                    self.last_hook_details.insert(s.id.clone(), detail);
                }
            }
        }
        // Load chains with link counts
        self.chains.clear();
        for name in self.store.list_chain_names().await.unwrap_or_default() {
            let count = self.store.get_chain_links(&name).await.map(|l| l.len()).unwrap_or(0);
            self.chains.push((name, count));
        }
        self.filter_chains();
        // Load artifacts
        self.artifacts = self.store.list_artifacts().await.unwrap_or_default();
        self.filter_artifacts();
        Ok(())
    }

    fn filter_chains(&mut self) {
        if self.chain_search.is_empty() {
            self.chains_filtered = self.chains.iter()
                .map(|(name, count)| (name.clone(), *count, 1.0))
                .collect();
        } else {
            let query = self.chain_search.to_lowercase();
            let mut scored: Vec<(String, usize, f64)> = self.chains.iter()
                .filter_map(|(name, count)| {
                    let n_lower = name.to_lowercase();
                    let base = jaro_winkler(&n_lower, &query);
                    let boost = if n_lower.contains(&query) { 0.3 } else { 0.0 };
                    let score = (base + boost).min(1.0);
                    if score > 0.4 { Some((name.clone(), *count, score)) } else { None }
                })
                .collect();
            scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
            self.chains_filtered = scored;
        }
        if self.chain_state.selected().map_or(false, |i| i >= self.chains_filtered.len()) {
            self.chain_state.select(if self.chains_filtered.is_empty() { None } else { Some(0) });
        }
    }

    fn filter_artifacts(&mut self) {
        if self.artifact_search.is_empty() {
            self.artifacts_filtered = self.artifacts.iter()
                .map(|a| (a.clone(), 1.0))
                .collect();
        } else {
            let query = self.artifact_search.to_lowercase();
            let mut scored: Vec<(Artifact, f64)> = self.artifacts.iter()
                .filter_map(|a| {
                    let searchable = format!("{} {}", a.title, a.description).to_lowercase();
                    let base = jaro_winkler(&searchable, &query);
                    let boost = if searchable.contains(&query) { 0.3 } else { 0.0 };
                    let score = (base + boost).min(1.0);
                    if score > 0.4 { Some((a.clone(), score)) } else { None }
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            self.artifacts_filtered = scored;
        }
        if self.artifact_state.selected().map_or(false, |i| i >= self.artifacts_filtered.len()) {
            self.artifact_state.select(if self.artifacts_filtered.is_empty() { None } else { Some(0) });
        }
    }

    async fn load_selected_chain(&mut self) {
        self.chain_scroll = 0;
        if let Some(i) = self.chain_state.selected() {
            if let Some((name, _, _)) = self.chains_filtered.get(i) {
                if let Ok(links) = self.store.get_chain_links(name).await {
                    self.chain_content = Some(self.format_chain_links(name, &links));
                    return;
                }
            }
        }
        self.chain_content = None;
    }

    async fn load_selected_artifact(&mut self) {
        self.artifact_scroll = 0;
        if let Some(i) = self.artifact_state.selected() {
            if let Some((artifact, _)) = self.artifacts_filtered.get(i) {
                let text = self.store.get_artifact_text(&artifact.id).await.ok().flatten();
                let ts = chrono::DateTime::from_timestamp(artifact.ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| artifact.ts.to_string());
                let content = format!(
                    "Title: {}\nType: {}\nPath: {}\nCreated: {}\nSession: {}\n\nDescription:\n{}\n\n{}",
                    artifact.title,
                    artifact.file_type.to_uppercase(),
                    artifact.file_path,
                    ts,
                    artifact.session_id,
                    artifact.description,
                    if let Some(t) = text {
                        format!("--- Extracted Text ---\n{}", if t.len() > 5000 { format!("{}...", &t[..5000]) } else { t })
                    } else {
                        "(no text extracted)".to_string()
                    }
                );
                self.artifact_content = Some(content);
                return;
            }
        }
        self.artifact_content = None;
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
                    match self.tab {
                        Tab::Chains => { self.chain_search.clear(); self.filter_chains(); }
                        Tab::Artifacts => { self.artifact_search.clear(); self.filter_artifacts(); }
                        _ => {}
                    }
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                    match self.tab {
                        Tab::Chains => self.load_selected_chain().await,
                        Tab::Artifacts => self.load_selected_artifact().await,
                        _ => {}
                    }
                }
                KeyCode::Backspace => {
                    match self.tab {
                        Tab::Chains => { self.chain_search.pop(); self.filter_chains(); }
                        Tab::Artifacts => { self.artifact_search.pop(); self.filter_artifacts(); }
                        _ => {}
                    }
                }
                KeyCode::Char(c) => {
                    match self.tab {
                        Tab::Chains => { self.chain_search.push(c); self.filter_chains(); }
                        Tab::Artifacts => { self.artifact_search.push(c); self.filter_artifacts(); }
                        _ => {}
                    }
                }
                _ => {}
            }
            return Ok(false);
        }
        if self.input_mode {
            match code {
                KeyCode::Esc => self.input_mode = false,
                KeyCode::Enter => {
                    self.input_mode = false;
                }
                KeyCode::Backspace => { self.input.pop(); }
                KeyCode::Char(c) => self.input.push(c),
                _ => {}
            }
        } else {
            match code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Tab => {
                    self.tab = match self.tab {
                        Tab::Active => Tab::Chains,
                        Tab::Chains => Tab::Artifacts,
                        Tab::Artifacts => Tab::History,
                        Tab::History => Tab::Active,
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.next();
                    match self.tab {
                        Tab::Chains => self.load_selected_chain().await,
                        Tab::Artifacts => self.load_selected_artifact().await,
                        _ => {}
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.prev();
                    match self.tab {
                        Tab::Chains => self.load_selected_chain().await,
                        Tab::Artifacts => self.load_selected_artifact().await,
                        _ => {}
                    }
                }
                KeyCode::Char('e') | KeyCode::Enter => {
                    match self.tab {
                        Tab::Chains => self.load_selected_chain().await,
                        Tab::Artifacts => self.load_selected_artifact().await,
                        _ => {}
                    }
                }
                KeyCode::Char('/') if matches!(self.tab, Tab::Chains | Tab::Artifacts) => {
                    self.search_mode = true;
                    match self.tab {
                        Tab::Chains => self.chain_search.clear(),
                        Tab::Artifacts => self.artifact_search.clear(),
                        _ => {}
                    }
                }
                KeyCode::Char('r') => self.refresh().await?,
                KeyCode::Char('d') => {
                    match self.tab {
                        Tab::Chains => self.delete_selected_chain().await?,
                        Tab::Artifacts => self.delete_selected_artifact().await?,
                        Tab::Active => self.archive_selected().await?,
                        _ => {}
                    }
                }
                // Scroll content panel
                KeyCode::Char('l') | KeyCode::PageDown if self.tab == Tab::Chains => {
                    self.chain_scroll = self.chain_scroll.saturating_add(5);
                }
                KeyCode::Char('h') | KeyCode::PageUp if self.tab == Tab::Chains => {
                    self.chain_scroll = self.chain_scroll.saturating_sub(5);
                }
                KeyCode::Char('l') | KeyCode::PageDown if self.tab == Tab::Artifacts => {
                    self.artifact_scroll = self.artifact_scroll.saturating_add(5);
                }
                KeyCode::Char('h') | KeyCode::PageUp if self.tab == Tab::Artifacts => {
                    self.artifact_scroll = self.artifact_scroll.saturating_sub(5);
                }
                _ => {}
            }
        }
        Ok(false)
    }

    async fn delete_selected_chain(&mut self) -> Result<()> {
        if let Some(i) = self.chain_state.selected() {
            if let Some((name, _, _)) = self.chains_filtered.get(i).cloned() {
                self.store.delete_chain(&name).await?;
                self.refresh().await?;
            }
        }
        Ok(())
    }

    async fn delete_selected_artifact(&mut self) -> Result<()> {
        if let Some(i) = self.artifact_state.selected() {
            if let Some((artifact, _)) = self.artifacts_filtered.get(i).cloned() {
                self.store.delete_artifact(&artifact.id).await?;
                self.refresh().await?;
            }
        }
        Ok(())
    }

    fn next(&mut self) {
        match self.tab {
            Tab::Active => {
                let i = self.session_state.selected()
                    .map(|i| (i + 1).min(self.sessions.len().saturating_sub(1)))
                    .unwrap_or(0);
                self.session_state.select(Some(i));
            }
            Tab::Chains => {
                let i = self.chain_state.selected()
                    .map(|i| (i + 1).min(self.chains_filtered.len().saturating_sub(1)))
                    .unwrap_or(0);
                self.chain_state.select(Some(i));
            }
            Tab::Artifacts => {
                let i = self.artifact_state.selected()
                    .map(|i| (i + 1).min(self.artifacts_filtered.len().saturating_sub(1)))
                    .unwrap_or(0);
                self.artifact_state.select(Some(i));
            }
            Tab::History => {}
        }
    }

    fn prev(&mut self) {
        match self.tab {
            Tab::Active => {
                let i = self.session_state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
                self.session_state.select(Some(i));
            }
            Tab::Chains => {
                let i = self.chain_state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
                self.chain_state.select(Some(i));
            }
            Tab::Artifacts => {
                let i = self.artifact_state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
                self.artifact_state.select(Some(i));
            }
            Tab::History => {}
        }
    }

    async fn archive_selected(&mut self) -> Result<()> {
        if self.tab == Tab::Active {
            if let Some(i) = self.session_state.selected() {
                if let Some(s) = self.sessions.get(i) {
                    self.store.mark_done(&s.id).await?;
                    self.refresh().await?;
                }
            }
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

        let chains_title = format!("Chains ({})", self.chains.len());
        let artifacts_title = format!("Artifacts ({})", self.artifacts.len());
        let titles: Vec<&str> = vec!["Active", &chains_title, &artifacts_title, "History"];
        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title(" tinymem "))
            .select(match self.tab {
                Tab::Active => 0,
                Tab::Chains => 1,
                Tab::Artifacts => 2,
                Tab::History => 3,
            })
            .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        f.render_widget(tabs, chunks[0]);

        match self.tab {
            Tab::Active => self.draw_active(f, chunks[1]),
            Tab::Chains => self.draw_chains(f, chunks[1]),
            Tab::Artifacts => self.draw_artifacts(f, chunks[1]),
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
                    (Status::Active, Some(tool)) => (
                        format!("RUNNING: {tool}"),
                        "\n\nTool in progress".to_string()
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

    fn draw_chains(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        let title = if self.search_mode {
            format!(" [/{}] ", self.chain_search)
        } else if !self.chain_search.is_empty() {
            format!(" (filter: {}) ", self.chain_search)
        } else {
            " [/] search ".into()
        };

        let items: Vec<ListItem> = self.chains_filtered.iter()
            .map(|(name, count, score)| {
                let score_str = if *score < 1.0 { format!(" ({:.0}%)", score * 100.0) } else { String::new() };
                ListItem::new(Line::from(vec![
                    Span::styled("🔗 ", Style::default().fg(Color::Cyan)),
                    Span::raw(name),
                    Span::styled(format!(" [{}]", count), Style::default().dim()),
                    Span::styled(score_str, Style::default().dim()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, chunks[0], &mut self.chain_state);

        let content = self.chain_content.as_deref().unwrap_or("Select a chain to view");
        let scroll_info = if self.chain_scroll > 0 { format!(" Content [^{}] ", self.chain_scroll) } else { " Content [h/l] ".into() };
        let p = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title(scroll_info))
            .wrap(Wrap { trim: false })
            .scroll((self.chain_scroll, 0));
        f.render_widget(p, chunks[1]);
    }

    fn draw_artifacts(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        let title = if self.search_mode {
            format!(" [/{}] ", self.artifact_search)
        } else if !self.artifact_search.is_empty() {
            format!(" (filter: {}) ", self.artifact_search)
        } else {
            " [/] search ".into()
        };

        let items: Vec<ListItem> = self.artifacts_filtered.iter()
            .map(|(artifact, score)| {
                let score_str = if *score < 1.0 { format!(" ({:.0}%)", score * 100.0) } else { String::new() };
                let icon = match artifact.file_type.as_str() {
                    "pdf" => "📄",
                    "md" => "📝",
                    _ => "📁",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", icon), Style::default().fg(Color::Yellow)),
                    Span::raw(&artifact.title),
                    Span::styled(format!(" [{}]", artifact.file_type.to_uppercase()), Style::default().dim()),
                    Span::styled(score_str, Style::default().dim()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, chunks[0], &mut self.artifact_state);

        let content = self.artifact_content.as_deref().unwrap_or("Select an artifact to view");
        let scroll_info = if self.artifact_scroll > 0 { format!(" Content [^{}] ", self.artifact_scroll) } else { " Content [h/l] ".into() };
        let p = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title(scroll_info))
            .wrap(Wrap { trim: false })
            .scroll((self.artifact_scroll, 0));
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
        let search_text = match self.tab {
            Tab::Chains => &self.chain_search,
            Tab::Artifacts => &self.artifact_search,
            _ => "",
        };
        let help = if self.search_mode {
            format!(" Search: {}_ | [Enter] select | [Esc] clear ", search_text)
        } else if self.input_mode {
            format!(" Input: {}_ | [Enter] submit | [Esc] cancel ", self.input)
        } else if matches!(self.tab, Tab::Chains | Tab::Artifacts) {
            " [/] search | [j/k] navigate | [d]elete | [r]efresh | [q]uit ".into()
        } else {
            " [Tab] switch | [j/k] navigate | [d]one | [r]efresh | [q]uit ".into()
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
