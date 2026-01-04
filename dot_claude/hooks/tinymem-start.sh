#!/bin/bash
# tinymem session start hook - uses /start endpoint with Redis-backed mapping
input=$(cat)
cwd=$(echo "$input" | jq -r '.cwd')
claude_sid=$(echo "$input" | jq -r '.session_id // empty')
host="${TINYMEM_HOST:-localhost}"
port="${TINYMEM_PORT:-3000}"
auth="Authorization: Bearer $TINYMEM_TOKEN"

# Call /start endpoint - handles mapping lookup/creation in Redis
response=$(curl -s -X POST "http://$host:$port/start" \
  -H "$auth" -H "Content-Type: application/json" \
  -d "{\"claude_session_id\":\"$claude_sid\",\"agent\":\"claude-code\",\"cwd\":\"$cwd\"}")

tinymem_sid=$(echo "$response" | jq -r '.id')

if [ -n "$tinymem_sid" ] && [ "$tinymem_sid" != "null" ]; then
  # Set env var for other hooks via Claude's env file (per-instance)
  if [ -n "$CLAUDE_ENV_FILE" ]; then
    echo "export TINYMEM_SESSION=$tinymem_sid" >> "$CLAUDE_ENV_FILE"
  fi
  # Write session-specific file (keyed by claude session id for multi-instance support)
  mkdir -p "$cwd/.claude" 2>/dev/null
  echo "$tinymem_sid" > "$cwd/.claude/.tinymem_session_$claude_sid" 2>/dev/null
fi
