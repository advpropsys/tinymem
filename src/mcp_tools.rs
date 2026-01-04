use serde_json::{json, Value};

/// Returns the list of MCP tools with their schemas
pub fn tool_list() -> Value {
    json!({
        "tools": [
            tool_ask(),
            tool_msg(),
            tool_save(),
            tool_search(),
            tool_get(),
            // Chain tools
            tool_chain_link(),
            tool_chain_load(),
            tool_chain_list(),
            tool_chain_search(),
        ]
    })
}

fn tool_ask() -> Value {
    json!({
        "name": "tinymem_ask",
        "description": "Ask a question to the user via tinymem TUI. Blocks until answered.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID (from TINYMEM_SESSION env)"
                },
                "question": {
                    "type": "string",
                    "description": "Question to ask the user"
                }
            },
            "required": ["session_id", "question"]
        }
    })
}

fn tool_msg() -> Value {
    json!({
        "name": "tinymem_msg",
        "description": "Send a message/note to the tinymem session log.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID"
                },
                "content": {
                    "type": "string",
                    "description": "Message content"
                }
            },
            "required": ["session_id", "content"]
        }
    })
}

fn tool_save() -> Value {
    json!({
        "name": "tinymem_save",
        "description": r#"Save a memory to persistent storage for later retrieval.

Use descriptive, searchable keys following this pattern:
- Lowercase with underscores (e.g. 'auth_jwt_refresh_pattern')
- 3-6 descriptive words capturing the essence
- Include domain context (e.g. 'react_', 'postgres_', 'api_')

Examples of good keys:
- 'auth_jwt_token_refresh_flow'
- 'react_useeffect_async_cleanup'
- 'postgres_connection_pool_config'
- 'error_handling_retry_logic'
- 'api_rate_limiting_middleware'

The key is used for fuzzy search, so make it descriptive!"#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID"
                },
                "key": {
                    "type": "string",
                    "description": "Descriptive key for fuzzy search (lowercase, underscores, 3-6 words)"
                },
                "content": {
                    "type": "string",
                    "description": "Memory content (insight, code, pattern, etc.)"
                },
                "kind": {
                    "type": "string",
                    "description": "Type of memory: insight, code, message, pattern",
                    "enum": ["insight", "code", "message", "pattern"],
                    "default": "insight"
                }
            },
            "required": ["session_id", "key", "content"]
        }
    })
}

fn tool_search() -> Value {
    json!({
        "name": "tinymem_search",
        "description": r#"Fuzzy search memories by key. Returns top matching keys sorted by relevance.

Use this to find relevant memories before retrieving them with tinymem_get.
The search uses fuzzy matching, so partial matches and typos are tolerated.

Returns: Array of {key, score} objects where score is 0-1 relevance."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (fuzzy matched against memory keys)"
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
        "description": r#"Retrieve a memory by exact key.

Use tinymem_search first to find relevant keys, then retrieve with this tool.
Returns the full memory content including metadata (kind, session_id, timestamp)."#,
        "inputSchema": {
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Exact memory key (from search results)"
                }
            },
            "required": ["key"]
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
