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
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook { pub ts: i64, pub kind: String, pub task: String, #[serde(default)] pub meta: Value }

#[derive(Debug, Deserialize)]
pub struct CreateSessionReq { pub agent: String, pub name: Option<String>, #[serde(default)] pub cwd: String }

#[derive(Debug, Deserialize)]
pub struct HookReq { pub kind: String, pub task: String, #[serde(default)] pub meta: Value }

#[derive(Debug, Deserialize)]
pub struct StartReq { pub claude_session_id: String, pub agent: String, #[serde(default)] pub cwd: String }

#[derive(Debug, Clone)]
pub enum TuiEvent { NewSession, SessionDone, Refresh }

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

// Artifact system - file references with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,              // unique id: {timestamp}_{sanitized_title}
    pub file_path: String,       // absolute path to file
    pub title: String,           // user-provided title
    pub description: String,     // user-provided description
    pub session_id: String,      // session that created it
    pub file_type: String,       // pdf, txt, md, etc.
    pub ts: i64,
}

#[derive(Debug, Deserialize)]
pub struct ArtifactSaveReq {
    pub file_path: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct GlobalSearchReq {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub result_type: String,     // "chain_link" or "artifact"
    pub id: String,              // chain:name:slug or artifact:id
    pub title: String,           // chain_name/slug or artifact title
    pub score: f64,
    pub preview: String,         // first ~200 chars of content
}

pub fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 }

pub fn short_id() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{:x}", t as u32 ^ (t >> 32) as u32)[..6].to_string()
}
