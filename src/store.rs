use anyhow::Result;
use redis::AsyncCommands;
use strsim::jaro_winkler;
use crate::models::{ChainLink, Hook, Memory, Msg, Session, Status};

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

    pub async fn add_msg(&self, id: &str, msg: &Msg) -> Result<()> {
        let mut conn = self.conn.clone();
        conn.rpush::<_, _, ()>(format!("sessions:{id}:msgs"), serde_json::to_string(msg)?).await?;
        Ok(())
    }

    pub async fn get_msgs(&self, id: &str, limit: isize) -> Result<Vec<Msg>> {
        let mut conn = self.conn.clone();
        let items: Vec<String> = conn.lrange(format!("sessions:{id}:msgs"), -limit, -1).await?;
        Ok(items.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    pub async fn set_pending(&self, id: &str, q: &str) -> Result<()> {
        self.conn.clone().set::<_, _, ()>(format!("sessions:{id}:pending"), q).await?; Ok(())
    }

    pub async fn get_pending(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.clone().get(format!("sessions:{id}:pending")).await?)
    }

    pub async fn set_answer(&self, id: &str, a: &str) -> Result<()> {
        self.conn.clone().set::<_, _, ()>(format!("sessions:{id}:answer"), a).await?; Ok(())
    }

    pub async fn get_answer(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.clone().get(format!("sessions:{id}:answer")).await?)
    }

    pub async fn clear_pending(&self, id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        redis::pipe().del(format!("sessions:{id}:pending")).del(format!("sessions:{id}:answer"))
            .query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn set_summary(&self, id: &str, summary: &str) -> Result<()> {
        self.conn.clone().set::<_, _, ()>(format!("sessions:{id}:summary"), summary).await?; Ok(())
    }

    pub async fn get_summary(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.clone().get(format!("sessions:{id}:summary")).await?)
    }

    pub async fn list_active(&self) -> Result<Vec<String>> { Ok(self.conn.clone().smembers("active").await?) }

    pub async fn list_history(&self, limit: isize) -> Result<Vec<String>> {
        Ok(self.conn.clone().lrange("history", 0, limit - 1).await?)
    }

    pub async fn list_waiting(&self) -> Result<Vec<(String, String)>> {
        let mut waiting = Vec::new();
        for id in self.list_active().await? {
            if let Ok(Some(q)) = self.get_pending(&id).await { waiting.push((id, q)); }
        }
        Ok(waiting)
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

    // Memory operations
    pub async fn save_memory(&self, mem: &Memory) -> Result<()> {
        let mut conn = self.conn.clone();
        redis::pipe()
            .set(format!("memories:{}", mem.key), serde_json::to_string(mem)?)
            .sadd("memory_keys", &mem.key)
            .query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn get_memory(&self, key: &str) -> Result<Option<Memory>> {
        let json: Option<String> = self.conn.clone().get(format!("memories:{key}")).await?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    pub async fn list_memory_keys(&self) -> Result<Vec<String>> {
        Ok(self.conn.clone().smembers("memory_keys").await?)
    }

    pub async fn delete_memory(&self, key: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        redis::pipe()
            .del(format!("memories:{key}"))
            .srem("memory_keys", key)
            .query_async::<()>(&mut conn).await?;
        Ok(())
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

    pub async fn get_chain_links_by_session(&self, session_id: &str) -> Result<Vec<ChainLink>> {
        // Get all chain names, then filter links by session_id
        let mut all_links = Vec::new();
        for name in self.list_chain_names().await? {
            for link in self.get_chain_links(&name).await? {
                if link.session_id == session_id {
                    all_links.push(link);
                }
            }
        }
        all_links.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(all_links)
    }
}
