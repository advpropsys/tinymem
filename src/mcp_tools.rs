use serde_json::{json, Value};

/// Returns the list of MCP tools with their schemas
pub fn tool_list() -> Value {
    json!({
        "tools": [
            tool_search(),
            tool_get(),
            tool_artifact_save(),
            // Chain tools
            tool_chain_link(),
            tool_chain_load(),
            tool_chain_list(),
            tool_chain_search(),
        ]
    })
}

fn tool_search() -> Value {
    json!({
        "name": "tinymem_search",
        "description": r#"Global search across all tinymem content - chains and artifacts.

Searches chain links (name, slug, content) and artifacts (title, description, extracted text).
Returns results sorted by relevance with type, id, title, score, and preview.

Use tinymem_get with the returned id to retrieve full content."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (matched against all content)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return",
                    "default": 25
                }
            },
            "required": ["query"]
        }
    })
}

fn tool_get() -> Value {
    json!({
        "name": "tinymem_get",
        "description": r#"Retrieve content by id from search results.

Supports two id formats:
- chain:name:slug - retrieves specific chain link content
- artifact:id - retrieves artifact with extracted text (for PDFs) or file content

Use tinymem_search first to find relevant ids."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Content id from search results (chain:name:slug or artifact:id)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return for text content (default: 8000). Use for large PDFs to avoid context overflow.",
                    "default": 8000
                },
                "offset": {
                    "type": "integer",
                    "description": "Character offset to start from (default: 0). Use with max_chars to paginate through large content.",
                    "default": 0
                }
            },
            "required": ["id"]
        }
    })
}

fn tool_artifact_save() -> Value {
    json!({
        "name": "tinymem_artifact_save",
        "description": r#"Save a file artifact to tinymem for later retrieval and search.

ONLY save artifacts that are genuinely useful for future reference:
- Research papers, technical docs, specifications
- Important configs, scripts, data files
- Reference materials the user explicitly wants to keep

DO NOT save: temporary files, build outputs, logs, or trivial files.

Title and description are critical for fuzzy search - provide meaningful metadata:
- Title: Descriptive name with key terms (e.g., "Adaptive Test-Time Compute Paper" not "2512.01457v4.pdf")
- Description: Key topics, authors, purpose - what would you search for to find this?

For PDFs, text is extracted and indexed. For text files, content is indexed directly.
The file stays on the filesystem - tinymem only stores the reference."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID (from TINYMEM_SESSION env)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "title": {
                    "type": "string",
                    "description": "Descriptive title with key search terms (NOT just filename). Example: 'Zero-Overhead Introspection for Adaptive Test-Time Compute' not '2512.01457v4'"
                },
                "description": {
                    "type": "string",
                    "description": "Key topics, authors, purpose - metadata that helps fuzzy search find this artifact later"
                }
            },
            "required": ["session_id", "file_path", "title"]
        }
    })
}

// ============ Chain Tools ============

fn tool_chain_link() -> Value {
    json!({
        "name": "tinymem_chain_link",
        "description": r#"Save a chain link - a checkpoint of your current work for multi-session projects.

Chain links capture:
- Current work context and technical decisions
- Code changes with file paths and details
- Unresolved issues and attempted solutions
- Pending tasks and next steps

Use this when pausing work on a feature to preserve context for later sessions.
The chain_name groups related links together (e.g., 'auth-feature', 'bug-fix-123').
The slug describes this specific checkpoint (e.g., 'implement-jwt', 'fix-refresh-token').

Example usage:
- chain_name: 'user-auth'
- slug: 'jwt-middleware-complete'
- content: '## Completed\n- JWT validation middleware\n- Token refresh logic\n\n## Next Steps\n- Add rate limiting'"#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Tinymem session ID (from TINYMEM_SESSION env)"
                },
                "chain_name": {
                    "type": "string",
                    "description": "Chain identifier (e.g., 'my-feature', 'bug-fix-123')"
                },
                "slug": {
                    "type": "string",
                    "description": "Short description of this checkpoint (e.g., 'implement-auth', 'fix-validation')"
                },
                "content": {
                    "type": "string",
                    "description": "Chain link content: context, decisions, code changes, next steps"
                }
            },
            "required": ["session_id", "chain_name", "slug", "content"]
        }
    })
}

fn tool_chain_load() -> Value {
    json!({
        "name": "tinymem_chain_load",
        "description": r#"Load chain links to continue work from a previous session.

Returns all links in the chain, sorted by timestamp (newest first).
Each link contains the preserved context, decisions, and next steps.

Use this at the start of a session to restore context from previous work.
The most recent link typically contains the immediate next steps."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "chain_name": {
                    "type": "string",
                    "description": "Chain identifier to load"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max links to return (default: 5)",
                    "default": 5
                }
            },
            "required": ["chain_name"]
        }
    })
}

fn tool_chain_list() -> Value {
    json!({
        "name": "tinymem_chain_list",
        "description": r#"List all available chains with their link counts.

Returns chain names with metadata about each chain.
Use this to discover what chains exist before loading one."#,
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    })
}

fn tool_chain_search() -> Value {
    json!({
        "name": "tinymem_chain_search",
        "description": r#"Fuzzy search for chains by name.

Returns matching chain names sorted by relevance score.
Use this to find chains when you don't remember the exact name."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (fuzzy matched against chain names)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return",
                    "default": 10
                }
            },
            "required": ["query"]
        }
    })
}
