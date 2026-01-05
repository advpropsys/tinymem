# tinymem

Lightweight AI agent memory server with TUI. Provides observability and coordination across local and remote agent sessions.

## Features

- **Cross-session persistence**: Chains and artifacts survive compaction, restarts, and context limits
- **Multi-machine coordination**: Connect agents on different machines to shared Redis backend
- **Real-time TUI**: Monitor all agent activity from a central dashboard
- **MCP integration**: Native Claude Code support via MCP protocol
- **Artifact management**: Store file references with automatic PDF text extraction

## Quick Start

```bash
# Start Redis
redis-server &

# Run tinymem
cargo run -- --token "your-secret-token" --port 3000
```

## CLI Options

```text
--redis <URL>     Redis URL (default: redis://127.0.0.1:6379)
--port <PORT>     Server port (default: 3000)
--token <TOKEN>   Auth token (required, or set TINYMEM_TOKEN)
--host <HOST>     Host for MCP mode (default: localhost)
--headless        Run without TUI
--mcp             Run as MCP server (stdio, for Claude Code)
```

## TUI Controls

| Key       | Action                    |
|-----------|---------------------------|
| Tab       | Switch tabs (Chains/Artifacts) |
| j/k       | Navigate up/down          |
| d         | Delete selected item      |
| Enter     | View details              |
| r         | Refresh                   |
| q         | Quit                      |

## Installation

```bash
# Set environment variables first
export TINYMEM_TOKEN="your-secret-token"
export TINYMEM_PORT="3000"  # optional, defaults to 3000

# Install hooks and MCP config to your project
./install.sh /path/to/your/project
```

The installer will:
- Copy hook scripts to `.claude/hooks/`
- Merge hooks into existing `settings.json` or create new one
- Build release binary and configure `.mcp.json`

### Manual Setup

If you prefer manual setup, add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "tinymem": {
      "command": "/path/to/tinymem/target/release/tinymem",
      "args": ["--mcp", "--port", "3000", "--token", "your-token"]
    }
  }
}
```

Set environment variables in your shell:

```bash
export TINYMEM_TOKEN="your-secret-token"
export TINYMEM_SESSION="your-session-id"
```

## Skills (Slash Commands)

Skills provide convenient shortcuts for chain operations in Claude Code:

| Skill | Description |
|-------|-------------|
| `/chain-link [name] [slug]` | Save work checkpoint with context, decisions, next steps |
| `/chain-list [query]` | List all chains or search by name |
| `/chain-load [name]` | Load chain to restore context from previous work |

Skills are installed automatically to `.claude/skills/` by the installer.

### Proactive Context Loading

When working on related topics, Claude Code can proactively search and suggest relevant chains. Use `/chain-load` anytime during a session, not just at start.

## MCP Tools

### Chains: Workflow Checkpoints

Chains persist context across sessions. Each chain contains multiple links capturing progress.

| Tool | Description |
|------|-------------|
| `tinymem_chain_link` | Save checkpoint: chain_name, slug, content |
| `tinymem_chain_load` | Load chain links by name |
| `tinymem_chain_list` | List all chains with link counts |
| `tinymem_chain_search` | Fuzzy search chains by name |

Example usage:

```
chain_name: "auth-feature"
slug: "jwt-middleware-complete"
content: "## Completed\n- JWT validation\n\n## Next\n- Add refresh tokens"
```

### Artifacts: File References

Artifacts store file references with metadata. PDFs are automatically extracted for text search.

| Tool | Description |
|------|-------------|
| `tinymem_artifact_save` | Save artifact: file_path, title, description |

Artifacts are searchable by title, description, and extracted text content.

### Global Search and Retrieval

| Tool | Description |
|------|-------------|
| `tinymem_search` | Search across all chains and artifacts |
| `tinymem_get` | Retrieve content by id (chain:name:slug or artifact:id) |

The `tinymem_get` tool supports pagination for large content:

```
tinymem_get(id: "artifact:abc123", max_chars: 8000, offset: 0)
```

## Architecture

```text
+------------------------------------------+
|              TUI (ratatui)               |
|         Chains | Artifacts               |
+--------------------+---------------------+
                     | mpsc
+--------------------+---------------------+
|           HTTP Server (axum)             |
|         Bearer token auth                |
+--------------------+---------------------+
                     |
+--------------------+---------------------+
|               Redis                      |
|    chains, artifacts, sessions           |
+------------------------------------------+
```

## API Reference

All endpoints require `Authorization: Bearer <token>` header.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/chain/link` | Save chain link |
| GET | `/chain/:name` | Load chain links |
| GET | `/chains` | List all chains |
| POST | `/artifact/save` | Save artifact |
| GET | `/search?q=...` | Global search |
| GET | `/get/:id` | Get content by id |

## License

MIT
