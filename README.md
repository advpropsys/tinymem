# tinymem

Lightweight AI agent coordination server with TUI. Track sessions, handle questions, coordinate multiple agents across machines.

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

| Key       | Action                              |
|-----------|-------------------------------------|
| Tab       | Switch tabs (Active/Pending/History)|
| j/k       | Navigate up/down                    |
| y         | Answer "yes" to pending question    |
| n         | Answer "no" to pending question     |
| e/Enter   | Enter text input mode               |
| Esc       | Exit input mode                     |
| r         | Refresh                             |
| q         | Quit                                |

## Claude Code Integration

### Quick Install

```bash
# Install hooks to your project
./install.sh /path/to/your/project

# Set environment (add to ~/.bashrc or ~/.zshrc)
export TINYMEM_TOKEN="your-secret-token"
export TINYMEM_HOST="your-server-ip"  # optional, defaults to localhost
export TINYMEM_PORT="3000"            # optional, defaults to 3000
```

The installer will:
- Create `.claude/hooks/` with tinymem scripts
- Merge hooks into existing `settings.json` or create new one
- Make all scripts executable

### Manual Setup

If you prefer manual setup, copy `dot_claude/` contents to your project's `.claude/` directory.

### MCP Server (for ask/msg tools)

The install script automatically configures `~/.claude/mcp_servers.json` with the tinymem MCP server.

This gives Claude Code two tools:
- `tinymem_ask`: Ask user a question (blocks until answered in TUI)
- `tinymem_msg`: Log a message to the session

## API Reference

All endpoints require `Authorization: Bearer <token>` header.

| Method | Endpoint             | Description                                        |
|--------|----------------------|----------------------------------------------------|
| POST   | `/session`           | Create session `{"agent":"...", "cwd":"...", "name":"..."}` |
| GET    | `/session`           | List active sessions                               |
| GET    | `/session/:id`       | Get session details                                |
| POST   | `/session/:id/hook`  | Add hook `{"kind":"pre\|post", "task":"...", "meta":{}}` |
| POST   | `/session/:id/msg`   | Add message `{"role":"...", "content":"..."}`      |
| POST   | `/session/:id/ask`   | Ask user (blocks up to 5min) `{"question":"..."}`  |
| POST   | `/session/:id/summary` | Store summary (body = text)                      |
| POST   | `/session/:id/done`  | Mark session complete                              |

### Ask User Example

```bash
# From agent - blocks until user answers in TUI
response=$(curl -s -X POST "http://localhost:3000/session/$TINYMEM_SESSION/ask" \
  -H "Authorization: Bearer $TINYMEM_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"question":"Should I proceed with the refactoring?"}')

answer=$(echo "$response" | jq -r '.answer')
echo "User said: $answer"
```

## Architecture

```text
+------------------------------------------+
|              TUI (ratatui)               |
|  Active | Pending | History              |
+--------------------+---------------------+
                     | mpsc
+--------------------+---------------------+
|           HTTP Server (axum)             |
|         Bearer token auth                |
+--------------------+---------------------+
                     |
+--------------------+---------------------+
|               Redis                      |
|  sessions, hooks, msgs, pending/answer   |
+------------------------------------------+
```

## License

MIT
