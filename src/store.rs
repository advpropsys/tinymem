use anyhow::Result;
use redis::AsyncCommands;
use strsim::jaro_winkler;
use crate::models::{Artifact, ChainLink, Hook, SearchResult, Session, Status};

#[derive(Clone)]
pub struct Store { conn: redis::aio::ConnectionManager }

impl Store {
    pub async fn new(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        Ok(Self { conn: redis::aio::ConnectionManager::new(client).await? })
    }

    pub async fn create_session(&self, s: &Session) -> Result<()> {
        let mut conn = self.conn.clone();
        let json = serde_json::to_string(s)?;
        redis::pipe().set(format!("sessions:{}", s.id), &json).sadd("active", &s.id)
            .query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let mut conn = self.conn.clone();
        let json: Option<String> = conn.get(format!("sessions:{id}")).await?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    pub async fn update_status(&self, id: &str, status: &Status) -> Result<()> {
        if let Some(mut s) = self.get_session(id).await? {
            s.status = status.clone();
            let mut conn = self.conn.clone();
            conn.set::<_, _, ()>(format!("sessions:{id}"), serde_json::to_string(&s)?).await?;
        }
        Ok(())
    }

    pub async fn mark_done(&self, id: &str) -> Result<()> {
        self.update_status(id, &Status::Done).await?;
        let mut conn = self.conn.clone();
        redis::pipe().srem("active", id).lpush("history", id).query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn add_hook(&self, id: &str, hook: &Hook) -> Result<()> {
        let mut conn = self.conn.clone();
        conn.rpush::<_, _, ()>(format!("sessions:{id}:hooks"), serde_json::to_string(hook)?).await?;
        self.touch_and_reactivate(id).await?;
        Ok(())
    }

    pub async fn touch_and_reactivate(&self, id: &str) -> Result<()> {
        if let Some(mut s) = self.get_session(id).await? {
            s.last_activity = crate::models::now();
            if s.status == crate::models::Status::Done {
                s.status = crate::models::Status::Active;
                let mut conn = self.conn.clone();
                redis::pipe()
                    .lrem::<_, _>("history", 1, id)
                    .sadd::<_, _>("active", id)
                    .query_async::<()>(&mut conn).await?;
            }
            let mut conn = self.conn.clone();
            conn.set::<_, _, ()>(format!("sessions:{id}"), serde_json::to_string(&s)?).await?;
        }
        Ok(())
    }

    pub async fn cleanup_stale(&self, max_inactive_secs: i64) -> Result<Vec<String>> {
        let now = crate::models::now();
        let mut cleaned = Vec::new();
        for id in self.list_active().await? {
            if let Ok(Some(s)) = self.get_session(&id).await {
                let age = now - s.last_activity;
                if age > max_inactive_secs && s.status == crate::models::Status::Active {
                    self.mark_done(&id).await?;
                    cleaned.push(id);
                }
            }
        }
        Ok(cleaned)
    }

    pub async fn get_hooks(&self, id: &str, limit: isize) -> Result<Vec<Hook>> {
        let mut conn = self.conn.clone();
        let items: Vec<String> = conn.lrange(format!("sessions:{id}:hooks"), -limit, -1).await?;
        Ok(items.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    pub async fn list_active(&self) -> Result<Vec<String>> { Ok(self.conn.clone().smembers("active").await?) }

    pub async fn list_history(&self, limit: isize) -> Result<Vec<String>> {
        Ok(self.conn.clone().lrange("history", 0, limit - 1).await?)
    }

    pub async fn set_active_tool(&self, id: &str, tool: &str) -> Result<()> {
        self.conn.clone().set::<_, _, ()>(format!("sessions:{id}:active_tool"), tool).await?;
        Ok(())
    }

    pub async fn clear_active_tool(&self, id: &str) -> Result<()> {
        self.conn.clone().del::<_, ()>(format!("sessions:{id}:active_tool")).await?;
        Ok(())
    }

    pub async fn get_active_tool(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.clone().get(format!("sessions:{id}:active_tool")).await?)
    }

    // Map Claude session ID to tinymem session ID
    pub async fn set_claude_mapping(&self, claude_id: &str, tinymem_id: &str) -> Result<()> {
        self.conn.clone().set::<_, _, ()>(format!("claude:{claude_id}"), tinymem_id).await?;
        Ok(())
    }

    pub async fn get_claude_mapping(&self, claude_id: &str) -> Result<Option<String>> {
        Ok(self.conn.clone().get(format!("claude:{claude_id}")).await?)
    }

    // Chain operations - multi-session workflow chains
    pub async fn save_chain_link(&self, link: &ChainLink) -> Result<String> {
        let mut conn = self.conn.clone();
        // Key: chains:{chain_name}:{timestamp}
        let key = format!("chains:{}:{}", link.chain_name, link.ts);
        redis::pipe()
            .set(&key, serde_json::to_string(link)?)
            .sadd("chain_names", &link.chain_name)
            .sadd(format!("chain:{}:links", link.chain_name), &key)
            .query_async::<()>(&mut conn).await?;
        Ok(key)
    }

    pub async fn get_chain_links(&self, chain_name: &str) -> Result<Vec<ChainLink>> {
        let mut conn = self.conn.clone();
        let keys: Vec<String> = conn.smembers(format!("chain:{}:links", chain_name)).await?;
        let mut links = Vec::new();
        for key in keys {
            if let Ok(Some(json)) = conn.get::<_, Option<String>>(&key).await {
                if let Ok(link) = serde_json::from_str::<ChainLink>(&json) {
                    links.push(link);
                }
            }
        }
        // Sort by timestamp descending (newest first)
        links.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(links)
    }

    pub async fn list_chain_names(&self) -> Result<Vec<String>> {
        Ok(self.conn.clone().smembers("chain_names").await?)
    }

    pub async fn search_chains(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        let names = self.list_chain_names().await?;
        let query_lower = query.to_lowercase();
        let mut scored: Vec<(String, f64)> = names.iter()
            .map(|n| {
                let n_lower = n.to_lowercase();
                let base = jaro_winkler(&n_lower, &query_lower);
                // Boost for substring match
                let boost = if n_lower.contains(&query_lower) { 0.3 } else { 0.0 };
                (n.clone(), (base + boost).min(1.0))
            })
            .filter(|(_, score)| *score > 0.4)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.truncate(limit);
        Ok(scored)
    }

    pub async fn delete_chain(&self, chain_name: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let link_keys: Vec<String> = conn.smembers(format!("chain:{}:links", chain_name)).await?;
        let mut pipe = redis::pipe();
        for key in &link_keys {
            pipe.del(key);
        }
        pipe.del(format!("chain:{}:links", chain_name));
        pipe.srem("chain_names", chain_name);
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    // Get specific chain link by chain_name and slug or timestamp
    pub async fn get_chain_link(&self, chain_name: &str, identifier: &str) -> Result<Option<ChainLink>> {
        let links = self.get_chain_links(chain_name).await?;
        // Try matching by slug first, then by timestamp
        Ok(links.into_iter().find(|l| l.slug == identifier || l.ts.to_string() == identifier))
    }

    // Artifact operations
    pub async fn save_artifact(&self, artifact: &Artifact) -> Result<()> {
        let mut conn = self.conn.clone();
        redis::pipe()
            .set(format!("artifacts:{}", artifact.id), serde_json::to_string(artifact)?)
            .sadd("artifact_ids", &artifact.id)
            .query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn get_artifact(&self, id: &str) -> Result<Option<Artifact>> {
        let json: Option<String> = self.conn.clone().get(format!("artifacts:{id}")).await?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    pub async fn list_artifacts(&self) -> Result<Vec<Artifact>> {
        let mut conn = self.conn.clone();
        let ids: Vec<String> = conn.smembers("artifact_ids").await?;
        let mut artifacts = Vec::new();
        for id in ids {
            if let Ok(Some(json)) = conn.get::<_, Option<String>>(format!("artifacts:{id}")).await {
                if let Ok(artifact) = serde_json::from_str::<Artifact>(&json) {
                    artifacts.push(artifact);
                }
            }
        }
        artifacts.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(artifacts)
    }

    pub async fn delete_artifact(&self, id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        redis::pipe()
            .del(format!("artifacts:{id}"))
            .srem("artifact_ids", id)
            // Also delete cached text extraction if exists
            .del(format!("artifacts:{id}:text"))
            .query_async::<()>(&mut conn).await?;
        Ok(())
    }

    // Cache extracted text for artifact (for search)
    pub async fn set_artifact_text(&self, id: &str, text: &str) -> Result<()> {
        self.conn.clone().set::<_, _, ()>(format!("artifacts:{id}:text"), text).await?;
        Ok(())
    }

    pub async fn get_artifact_text(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.clone().get(format!("artifacts:{id}:text")).await?)
    }

    // Global search across chains and artifacts
    pub async fn global_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        // Search chain links
        for chain_name in self.list_chain_names().await? {
            for link in self.get_chain_links(&chain_name).await? {
                let searchable = format!("{} {} {}", chain_name, link.slug, link.content).to_lowercase();
                let score = self.compute_search_score(&searchable, &query_lower);
                if score > 0.3 {
                    let preview = link.content.chars().take(200).collect::<String>();
                    results.push(SearchResult {
                        result_type: "chain_link".to_string(),
                        id: format!("chain:{}:{}", chain_name, link.slug),
                        title: format!("{}/{}", chain_name, link.slug),
                        score,
                        preview,
                    });
                }
            }
        }

        // Search artifacts
        for artifact in self.list_artifacts().await? {
            let cached_text = self.get_artifact_text(&artifact.id).await?.unwrap_or_default();
            let searchable = format!("{} {} {}", artifact.title, artifact.description, cached_text).to_lowercase();
            let score = self.compute_search_score(&searchable, &query_lower);
            if score > 0.3 {
                let preview = if !cached_text.is_empty() {
                    cached_text.chars().take(200).collect()
                } else {
                    artifact.description.chars().take(200).collect()
                };
                results.push(SearchResult {
                    result_type: "artifact".to_string(),
                    id: format!("artifact:{}", artifact.id),
                    title: artifact.title.clone(),
                    score,
                    preview,
                });
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results.truncate(limit);
        Ok(results)
    }

    fn compute_search_score(&self, text: &str, query: &str) -> f64 {
        // Simple scoring: substring match gets high score, jaro-winkler for fuzzy
        if text.contains(query) {
            0.9 + (query.len() as f64 / text.len() as f64).min(0.1)
        } else {
            // Check individual words
            let words: Vec<&str> = query.split_whitespace().collect();
            let matches = words.iter().filter(|w| text.contains(*w)).count();
            if matches > 0 {
                0.5 + (matches as f64 / words.len() as f64) * 0.4
            } else {
                // Fall back to jaro-winkler on title-like portion
                let first_100: String = text.chars().take(100).collect();
                jaro_winkler(&first_100, query) * 0.5
            }
        }
    }
}
