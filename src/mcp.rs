use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use crate::mcp_tools;

#[derive(Deserialize)]
struct Request { id: Option<Value>, method: String, params: Option<Value> }

#[derive(Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

pub fn run(host: &str, port: u16, token: &str) {
    let base = format!("http://{}:{}", host, port);
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines().filter_map(|l| l.ok()) {
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let id = req.id.clone().unwrap_or(Value::Null);
        let result = handle(&req.method, req.params, &base, token);
        let resp = match result {
            Ok(r) => Response { jsonrpc: "2.0", id, result: Some(r), error: None },
            Err(e) => Response { jsonrpc: "2.0", id, result: None, error: Some(json!({"code": -1, "message": e})) },
        };
        let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
        let _ = stdout.flush();
    }
}

fn handle(method: &str, params: Option<Value>, base: &str, token: &str) -> Result<Value, String> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "tinymem", "version": "0.1.0" }
        })),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => Ok(mcp_tools::tool_list()),
        "tools/call" => {
            let p = params.ok_or("missing params")?;
            let name = p.get("name").and_then(|v| v.as_str()).ok_or("missing tool name")?;
            let args = p.get("arguments").cloned().unwrap_or(json!({}));
            call_tool(name, args, base, token)
        }
        _ => Ok(Value::Null)
    }
}

fn call_tool(name: &str, args: Value, base: &str, token: &str) -> Result<Value, String> {
    match name {
        "tinymem_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("missing query")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(25) as usize;
            let url = format!("{}/search", base);
            let mut resp = ureq::post(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .send_json(&json!({"query": query, "limit": limit}))
                .map_err(|e| format!("request failed: {}", e))?;
            let body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            let results = body.get("results").cloned().unwrap_or(json!([]));
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&results).unwrap()}]}))
        }
        "tinymem_get" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing id")?;
            let max_chars = args.get("max_chars").and_then(|v| v.as_u64()).unwrap_or(8000) as usize;
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let url = format!("{}/get/{}", base, urlencoding::encode(id));
            let mut resp = ureq::get(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .call()
                .map_err(|e| format!("request failed: {}", e))?;
            let mut body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            // Truncate text field for artifacts to avoid context overflow
            if let Some(text) = body.get("text").and_then(|v| v.as_str()) {
                let total_len = text.len();
                let chars: String = text.chars().skip(offset).take(max_chars).collect();
                let end_offset = offset + chars.len();
                body["text"] = json!(chars);
                body["text_range"] = json!({"offset": offset, "end": end_offset, "total": total_len});
                if end_offset < total_len {
                    body["has_more"] = json!(true);
                    body["next_offset"] = json!(end_offset);
                }
            }
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&body).unwrap()}]}))
        }
        "tinymem_artifact_save" => {
            let sid = args.get("session_id").and_then(|v| v.as_str()).ok_or("missing session_id")?;
            let file_path = args.get("file_path").and_then(|v| v.as_str()).ok_or("missing file_path")?;
            let title = args.get("title").and_then(|v| v.as_str()).ok_or("missing title")?;
            let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let url = format!("{}/artifact/save/{}", base, sid);
            let mut resp = ureq::post(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .send_json(&json!({"file_path": file_path, "title": title, "description": description}))
                .map_err(|e| format!("request failed: {}", e))?;
            let body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
            Ok(json!({"content": [{"type": "text", "text": format!("artifact saved: {}", id)}]}))
        }
        // Chain tools
        "tinymem_chain_link" => {
            let sid = args.get("session_id").and_then(|v| v.as_str()).ok_or("missing session_id")?;
            let chain_name = args.get("chain_name").and_then(|v| v.as_str()).ok_or("missing chain_name")?;
            let slug = args.get("slug").and_then(|v| v.as_str()).ok_or("missing slug")?;
            let content = args.get("content").and_then(|v| v.as_str()).ok_or("missing content")?;
            let url = format!("{}/chain/{}", base, sid);
            let mut resp = ureq::post(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .send_json(&json!({"chain_name": chain_name, "slug": slug, "content": content}))
                .map_err(|e| format!("request failed: {}", e))?;
            let body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            let saved = body.get("saved").and_then(|v| v.as_str()).unwrap_or("unknown");
            Ok(json!({"content": [{"type": "text", "text": format!("chain link saved: {}", saved)}]}))
        }
        "tinymem_chain_load" => {
            let chain_name = args.get("chain_name").and_then(|v| v.as_str()).ok_or("missing chain_name")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            let url = format!("{}/chain/get/{}", base, chain_name);
            let mut resp = ureq::get(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .call()
                .map_err(|e| format!("request failed: {}", e))?;
            let body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            let links = body.get("links").cloned().unwrap_or(json!([]));
            // Limit results
            let limited: Vec<Value> = links.as_array().map(|arr| arr.iter().take(limit).cloned().collect()).unwrap_or_default();
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&limited).unwrap()}]}))
        }
        "tinymem_chain_list" => {
            let url = format!("{}/chains", base);
            let mut resp = ureq::get(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .call()
                .map_err(|e| format!("request failed: {}", e))?;
            let body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            let chains = body.get("chains").cloned().unwrap_or(json!([]));
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&chains).unwrap()}]}))
        }
        "tinymem_chain_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("missing query")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let url = format!("{}/chain/search", base);
            let mut resp = ureq::post(&url)
                .header("Authorization", &format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .send_json(&json!({"query": query, "limit": limit}))
                .map_err(|e| format!("request failed: {}", e))?;
            let body: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            let chains = body.get("chains").cloned().unwrap_or(json!([]));
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&chains).unwrap()}]}))
        }
        _ => Err(format!("unknown tool: {}", name))
    }
}
