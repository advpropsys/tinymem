use anyhow::Result;
use axum::{body::Body, extract::{Path, Request, State}, http::{HeaderMap, StatusCode},
    middleware::{self, Next}, response::{IntoResponse, Response}, routing::post, Json, Router};
use serde_json::json;
use tokio::{net::TcpListener, sync::mpsc::Sender};
use crate::models::{now, short_id, Artifact, ArtifactSaveReq, ChainLink, ChainSaveReq, ChainSearchReq, CreateSessionReq, GlobalSearchReq, Hook, HookReq, Session, StartReq, Status, TuiEvent};
use crate::store::Store;
use std::path::Path as FilePath;

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

async fn mark_done(State(s): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let _ = s.tui_tx.send(TuiEvent::SessionDone).await;
    s.store.mark_done(&id).await.map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
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

// Global search endpoint
async fn global_search(State(s): State<AppState>, Json(r): Json<GlobalSearchReq>) -> impl IntoResponse {
    match s.store.global_search(&r.query, r.limit).await {
        Ok(results) => (StatusCode::OK, Json(json!({ "results": results }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

// Global get endpoint - handles chain:name:slug and artifact:id
async fn global_get(State(s): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let id = urlencoding::decode(&id).unwrap_or_default().to_string();
    if id.starts_with("chain:") {
        // Parse chain:name:slug
        let parts: Vec<&str> = id.splitn(3, ':').collect();
        if parts.len() >= 3 {
            let chain_name = parts[1];
            let slug = parts[2];
            match s.store.get_chain_link(chain_name, slug).await {
                Ok(Some(link)) => return (StatusCode::OK, Json(json!({
                    "type": "chain_link",
                    "chain_name": link.chain_name,
                    "slug": link.slug,
                    "content": link.content,
                    "session_id": link.session_id,
                    "ts": link.ts
                }))),
                Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "chain link not found" }))),
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
            }
        }
    } else if id.starts_with("artifact:") {
        let artifact_id = &id[9..];
        match s.store.get_artifact(artifact_id).await {
            Ok(Some(artifact)) => {
                let text = s.store.get_artifact_text(artifact_id).await.ok().flatten().unwrap_or_default();
                return (StatusCode::OK, Json(json!({
                    "type": "artifact",
                    "id": artifact.id,
                    "file_path": artifact.file_path,
                    "title": artifact.title,
                    "description": artifact.description,
                    "file_type": artifact.file_type,
                    "text": text,
                    "session_id": artifact.session_id,
                    "ts": artifact.ts
                })));
            }
            Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "artifact not found" }))),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
        }
    }
    (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid id format, expected chain:name:slug or artifact:id" })))
}

// Artifact endpoints
async fn save_artifact(State(s): State<AppState>, Path(session_id): Path<String>, Json(r): Json<ArtifactSaveReq>) -> impl IntoResponse {
    let path = FilePath::new(&r.file_path);
    if !path.exists() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "file not found" })));
    }
    let file_type = path.extension().and_then(|e| e.to_str()).unwrap_or("txt").to_lowercase();
    let ts = now();
    let sanitized_title: String = r.title.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').take(50).collect();
    let id = format!("{}_{}", ts, sanitized_title);

    let artifact = Artifact {
        id: id.clone(),
        file_path: r.file_path.clone(),
        title: r.title,
        description: r.description,
        session_id,
        file_type: file_type.clone(),
        ts,
    };

    // Extract text for indexing
    let text = extract_file_text(&r.file_path, &file_type);

    match s.store.save_artifact(&artifact).await {
        Ok(_) => {
            if !text.is_empty() {
                let _ = s.store.set_artifact_text(&id, &text).await;
            }
            (StatusCode::OK, Json(json!({ "id": id, "file_type": file_type })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn list_artifacts(State(s): State<AppState>) -> impl IntoResponse {
    match s.store.list_artifacts().await {
        Ok(artifacts) => (StatusCode::OK, Json(json!({ "artifacts": artifacts }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

async fn delete_artifact(State(s): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match s.store.delete_artifact(&id).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "deleted": id }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
    }
}

fn extract_file_text(file_path: &str, file_type: &str) -> String {
    match file_type {
        "pdf" => {
            match mupdf::Document::open(file_path) {
                Ok(doc) => {
                    let mut text = String::new();
                    let page_count = doc.page_count().unwrap_or(0);
                    for i in 0..page_count {
                        if let Ok(page) = doc.load_page(i) {
                            if let Ok(tp) = page.to_text_page(mupdf::TextPageFlags::empty()) {
                                for block in tp.blocks() {
                                    for line in block.lines() {
                                        for ch in line.chars() {
                                            if let Some(c) = ch.char() {
                                                text.push(c);
                                            }
                                        }
                                        text.push('\n');
                                    }
                                }
                            }
                        }
                        if text.len() > 50000 { break; }
                    }
                    text.chars().take(50000).collect()
                }
                Err(_) => String::new()
            }
        }
        "txt" | "md" | "json" | "yaml" | "yml" | "toml" | "rs" | "py" | "js" | "ts" => {
            std::fs::read_to_string(file_path)
                .map(|s| s.chars().take(50000).collect())
                .unwrap_or_default()
        }
        _ => String::new()
    }
}

pub async fn run(store: Store, token: String, tui_tx: Sender<TuiEvent>, port: u16) -> Result<()> {
    let state = AppState { store, tui_tx, token: token.clone() };
    let app = Router::new()
        .route("/session", post(create_session).get(list_sessions))
        .route("/start", post(start_session))
        .route("/session/:id", axum::routing::get(get_session))
        .route("/session/:id/hook", post(add_hook))
        .route("/session/:id/done", post(mark_done))
        // Chain endpoints
        .route("/chain/:session_id", post(save_chain_link))
        .route("/chain/get/:chain_name", axum::routing::get(get_chain_links))
        .route("/chains", axum::routing::get(list_chains))
        .route("/chain/search", post(search_chains))
        // Global search and get
        .route("/search", post(global_search))
        .route("/get/*id", axum::routing::get(global_get))
        // Artifact endpoints
        .route("/artifact/save/:session_id", post(save_artifact))
        .route("/artifacts", axum::routing::get(list_artifacts))
        .route("/artifact/delete/:id", axum::routing::delete(delete_artifact))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state);
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    eprintln!("Server listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
