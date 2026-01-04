use anyhow::Result;
use axum::{body::Body, extract::{Path, Request, State}, http::{HeaderMap, StatusCode},
    middleware::{self, Next}, response::{IntoResponse, Response}, routing::post, Json, Router};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::{net::TcpListener, sync::mpsc::Sender};
use crate::models::{now, short_id, AskReq, ChainLink, ChainSaveReq, ChainSearchReq, CreateSessionReq, Hook, HookReq, Memory, MemorySaveReq, MemorySearchReq, Msg, Session, StartReq, Status, TuiEvent};
use strsim::jaro_winkler;
use crate::store::Store;

#[derive(Clone)]
pub struct AppState { pub store: Store, pub tui_tx: Sender<TuiEvent>, pub token: String }

async fn auth(State(s): State<AppState>, h: HeaderMap, req: Request<Body>, next: Next) -> Response {
    let a = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    if a == format!("Bearer {}", s.token) || s.token.is_empty() { next.run(req).await }
    else { StatusCode::UNAUTHORIZED.into_response() }
}

async fn create_session(State(s): State<AppState>, Json(r): Json<CreateSessionReq>) -> impl IntoResponse {
    let id = r.name.clone().unwrap_or_else(short_id);
    let ts = now();
    let session = Session { id: id.clone(), name: r.name, agent: r.agent, cwd: r.cwd, status: Status::Active, created: ts, last_activity: ts };
    match s.store.create_session(&session).await {
        Ok(_) => { let _ = s.tui_tx.send(TuiEvent::NewSession).await; (StatusCode::OK, Json(json!({ "id": id }))) }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

// Start/resume session with Claude session ID mapping (stored in Redis)
async fn start_session(State(s): State<AppState>, Json(r): Json<StartReq>) -> impl IntoResponse {
    // Check for existing mapping
    if let Ok(Some(tinymem_id)) = s.store.get_claude_mapping(&r.claude_session_id).await {
        // Check if session exists
        if let Ok(Some(_)) = s.store.get_session(&tinymem_id).await {
            // Reactivate and return existing session
            let _ = s.store.touch_and_reactivate(&tinymem_id).await;
            let _ = s.tui_tx.send(TuiEvent::Refresh).await;
            return (StatusCode::OK, Json(json!({ "id": tinymem_id, "reused": true })));
        }
    }
    // Create new session
    let id = short_id();
    let ts = now();
    let session = Session { id: id.clone(), name: None, agent: r.agent, cwd: r.cwd, status: Status::Active, created: ts, last_activity: ts };
    match s.store.create_session(&session).await {
        Ok(_) => {
            let _ = s.store.set_claude_mapping(&r.claude_session_id, &id).await;
            let _ = s.tui_tx.send(TuiEvent::NewSession).await;
            (StatusCode::OK, Json(json!({ "id": id, "reused": false })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn add_hook(State(s): State<AppState>, Path(id): Path<String>, Json(r): Json<HookReq>) -> StatusCode {
    let hook = Hook { ts: now(), kind: r.kind.clone(), task: r.task.clone(), meta: r.meta };
    // Track active tool for TUI display
    if r.kind == "pre" {
        let _ = s.store.set_active_tool(&id, &r.task).await;
    } else {
        let _ = s.store.clear_active_tool(&id).await;
    }
    let _ = s.tui_tx.send(TuiEvent::Refresh).await; // Notify TUI
    s.store.add_hook(&id, &hook).await.map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn add_msg(State(s): State<AppState>, Path(id): Path<String>, Json(r): Json<Msg>) -> StatusCode {
    let msg = Msg { ts: now(), role: r.role, content: r.content };
    s.store.add_msg(&id, &msg).await.map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn set_summary(State(s): State<AppState>, Path(id): Path<String>, body: String) -> StatusCode {
    s.store.add_msg(&id, &Msg { ts: now(), role: "summary".into(), content: body }).await
        .map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn mark_done(State(s): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let _ = s.tui_tx.send(TuiEvent::SessionDone).await;
    s.store.mark_done(&id).await.map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn ask_user(State(s): State<AppState>, Path(id): Path<String>, Json(r): Json<AskReq>) -> impl IntoResponse {
    let timeout = Duration::from_secs(300);
    let start = Instant::now();
    // Touch and reactivate session if it was marked done
    let _ = s.store.touch_and_reactivate(&id).await;
    // Clear any stale answer from previous question
    let _ = s.store.clear_pending(&id).await;
    if let Err(e) = s.store.set_pending(&id, &r.question).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
    }
    let _ = s.store.update_status(&id, &Status::Waiting { question: r.question.clone(), asked_at: now() }).await;
    let _ = s.tui_tx.send(TuiEvent::NewQuestion).await;
    loop {
        if start.elapsed() > timeout {
            let _ = s.store.clear_pending(&id).await;
            let _ = s.store.update_status(&id, &Status::Active).await;
            return (StatusCode::REQUEST_TIMEOUT, Json(json!({ "error": "timeout" })));
        }
        if let Ok(Some(answer)) = s.store.get_answer(&id).await {
            let _ = s.store.clear_pending(&id).await;
            let _ = s.store.update_status(&id, &Status::Active).await;
            return (StatusCode::OK, Json(json!({ "answer": answer })));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn get_session(State(s): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match s.store.get_session(&id).await {
        Ok(Some(sess)) => (StatusCode::OK, Json(json!(sess))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn list_sessions(State(s): State<AppState>) -> impl IntoResponse {
    match s.store.list_active().await {
        Ok(ids) => (StatusCode::OK, Json(json!({ "sessions": ids }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

// Memory endpoints
async fn save_memory(State(s): State<AppState>, Path(session_id): Path<String>, Json(r): Json<MemorySaveReq>) -> impl IntoResponse {
    let mem = Memory {
        key: r.key.clone(),
        session_id,
        content: r.content,
        kind: if r.kind.is_empty() { "insight".into() } else { r.kind },
        ts: now(),
    };
    match s.store.save_memory(&mem).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "saved": r.key }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn search_memory(State(s): State<AppState>, Json(r): Json<MemorySearchReq>) -> impl IntoResponse {
    match s.store.list_memory_keys().await {
        Ok(keys) => {
            let query_lower = r.query.to_lowercase();
            let mut scored: Vec<(String, f64)> = keys.iter()
                .map(|k| {
                    let k_lower = k.to_lowercase();
                    let base = jaro_winkler(&k_lower, &query_lower);
                    let boost = if k_lower.contains(&query_lower) { 0.2 } else { 0.0 };
                    (k.clone(), (base + boost).min(1.0))
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            scored.truncate(r.limit);
            let results: Vec<_> = scored.into_iter().map(|(k, score)| json!({"key": k, "score": score})).collect();
            (StatusCode::OK, Json(json!({ "keys": results })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn get_memory(State(s): State<AppState>, Path(key): Path<String>) -> impl IntoResponse {
    match s.store.get_memory(&key).await {
        Ok(Some(mem)) => (StatusCode::OK, Json(json!({ "memory": mem }))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn delete_memory(State(s): State<AppState>, Path(key): Path<String>) -> impl IntoResponse {
    match s.store.delete_memory(&key).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "deleted": key }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

// Chain endpoints
async fn save_chain_link(State(s): State<AppState>, Path(session_id): Path<String>, Json(r): Json<ChainSaveReq>) -> impl IntoResponse {
    let link = ChainLink {
        chain_name: r.chain_name.clone(),
        session_id,
        slug: r.slug.clone(),
        content: r.content,
        ts: now(),
    };
    match s.store.save_chain_link(&link).await {
        Ok(key) => (StatusCode::OK, Json(json!({ "saved": key, "chain": r.chain_name, "slug": r.slug }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn get_chain_links(State(s): State<AppState>, Path(chain_name): Path<String>) -> impl IntoResponse {
    match s.store.get_chain_links(&chain_name).await {
        Ok(links) => (StatusCode::OK, Json(json!({ "chain": chain_name, "links": links, "count": links.len() }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn list_chains(State(s): State<AppState>) -> impl IntoResponse {
    match s.store.list_chain_names().await {
        Ok(names) => {
            // Get link count for each chain
            let mut chains = Vec::new();
            for name in names {
                let count = s.store.get_chain_links(&name).await.map(|l| l.len()).unwrap_or(0);
                chains.push(json!({ "name": name, "links": count }));
            }
            (StatusCode::OK, Json(json!({ "chains": chains })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn search_chains(State(s): State<AppState>, Json(r): Json<ChainSearchReq>) -> impl IntoResponse {
    match s.store.search_chains(&r.query, r.limit).await {
        Ok(results) => {
            let chains: Vec<_> = results.into_iter().map(|(name, score)| json!({"name": name, "score": score})).collect();
            (StatusCode::OK, Json(json!({ "chains": chains })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

pub async fn run(store: Store, token: String, tui_tx: Sender<TuiEvent>, port: u16) -> Result<()> {
    let state = AppState { store, tui_tx, token: token.clone() };
    let app = Router::new()
        .route("/session", post(create_session).get(list_sessions))
        .route("/start", post(start_session))
        .route("/session/:id", axum::routing::get(get_session))
        .route("/session/:id/hook", post(add_hook))
        .route("/session/:id/ask", post(ask_user))
        .route("/session/:id/msg", post(add_msg))
        .route("/session/:id/summary", post(set_summary))
        .route("/session/:id/done", post(mark_done))
        // Memory endpoints
        .route("/memory/:session_id", post(save_memory))
        .route("/memory/search", post(search_memory))
        .route("/memory/get/:key", axum::routing::get(get_memory))
        .route("/memory/delete/:key", post(delete_memory))
        // Chain endpoints
        .route("/chain/:session_id", post(save_chain_link))
        .route("/chain/get/:chain_name", axum::routing::get(get_chain_links))
        .route("/chains", axum::routing::get(list_chains))
        .route("/chain/search", post(search_chains))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state);
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    eprintln!("Server listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
