#!/bin/bash
# tinymem pre/post tool hook

input=$(cat)

# Get tinymem session: prefer env, then session-specific file
if [ -z "$TINYMEM_SESSION" ]; then
  claude_sid=$(echo "$input" | jq -r '.session_id // empty')
  if [ -n "$claude_sid" ] && [ -f ".claude/.tinymem_session_$claude_sid" ]; then
    TINYMEM_SESSION=$(cat ".claude/.tinymem_session_$claude_sid")
  fi
fi

[ -z "$TINYMEM_SESSION" ] && exit 0

tool=$(echo "$input" | jq -r '.tool_name')
event=$(echo "$input" | jq -r '.hook_event_name')

[[ "$tool" == *tinymem* ]] && exit 0

kind=$([[ "$event" == "PreToolUse" ]] && echo "pre" || echo "post")

# Build JSON payload properly using jq to avoid escaping issues
payload=$(echo "$input" | jq -c --arg kind "$kind" --arg task "$tool" '{kind: $kind, task: $task, meta: (.tool_input // {})}')

curl -s --max-time 2 -X POST "http://${TINYMEM_HOST:-localhost}:${TINYMEM_PORT:-3000}/session/$TINYMEM_SESSION/hook" \
  -H "Authorization: Bearer $TINYMEM_TOKEN" \
  -H "Content-Type: application/json" \
  -d "$payload" > /dev/null 2>&1 &
disown
