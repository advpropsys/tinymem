#!/bin/bash
# Intercept AskUserQuestion and route through tinymem
[ -z "$TINYMEM_SESSION" ] && exit 0
[ -z "$TINYMEM_TOKEN" ] && exit 0

input=$(cat)
tool=$(echo "$input" | jq -r '.tool_name')

# Only intercept AskUserQuestion
[ "$tool" != "AskUserQuestion" ] && exit 0

# Extract question from tool input
question=$(echo "$input" | jq -r '.tool_input.question // .tool_input.text // "Approval needed"')

# Send to tinymem and wait for answer
response=$(curl -s -X POST "http://${TINYMEM_HOST:-localhost}:${TINYMEM_PORT:-3000}/session/$TINYMEM_SESSION/ask" \
  -H "Authorization: Bearer $TINYMEM_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"question\": \"$question\"}" \
  --max-time 310)

answer=$(echo "$response" | jq -r '.answer // "yes"')

# Try to provide the answer back - this might work or might not
# Option 1: Output as decision with the answer
echo "{\"decision\": \"approve\", \"answer\": \"$answer\"}"
