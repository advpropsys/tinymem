use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String, pub name: Option<String>, pub agent: String,
    pub cwd: String, pub status: Status, pub created: i64,
    #[serde(default)] pub last_activity: i64, // defaults to 0 for old sessions
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type")]
pub enum Status {
    #[default] Active,
    Waiting { question: String, asked_at: i64 },
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook { pub ts: i64, pub kind: String, pub task: String, #[serde(default)] pub meta: Value }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Msg { pub ts: i64, pub role: String, pub content: String }

#[derive(Debug, Deserialize)]
pub struct CreateSessionReq { pub agent: String, pub name: Option<String>, #[serde(default)] pub cwd: String }

#[derive(Debug, Deserialize)]
pub struct AskReq { pub question: String }

#[derive(Debug, Deserialize)]
pub struct HookReq { pub kind: String, pub task: String, #[serde(default)] pub meta: Value }

#[derive(Debug, Deserialize)]
pub struct StartReq { pub claude_session_id: String, pub agent: String, #[serde(default)] pub cwd: String }

#[derive(Debug, Clone)]
pub enum TuiEvent { NewSession, NewQuestion, SessionDone, Refresh }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub key: String,
    pub session_id: String,
    pub content: String,
    pub kind: String,  // insight, code, message, pattern
    pub ts: i64,
}

#[derive(Debug, Deserialize)]
pub struct MemorySaveReq { pub key: String, pub content: String, #[serde(default)] pub kind: String }

#[derive(Debug, Deserialize)]
pub struct MemorySearchReq { pub query: String, #[serde(default = "default_limit")] pub limit: usize }

fn default_limit() -> usize { 25 }

// Chain system - multi-session workflow chains
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainLink {
    pub chain_name: String,      // e.g., "my-feature"
    pub session_id: String,      // tinymem session that created it
    pub slug: String,            // e.g., "implement-auth"
    pub content: String,         // the chain link content (analysis, context, next steps)
    pub ts: i64,                 // timestamp
}

#[derive(Debug, Deserialize)]
pub struct ChainSaveReq {
    pub chain_name: String,
    pub slug: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ChainSearchReq {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

pub fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 }

pub fn short_id() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{:x}", t as u32 ^ (t >> 32) as u32)[..6].to_string()
}
