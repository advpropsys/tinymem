#!/bin/bash
# tinymem session end hook

# Get session from project's .claude dir
if [ -z "$TINYMEM_SESSION" ] && [ -f ".claude/.tinymem_session" ]; then
  TINYMEM_SESSION=$(cat ".claude/.tinymem_session")
fi

[ -z "$TINYMEM_SESSION" ] && exit 0

curl -s -X POST "http://${TINYMEM_HOST:-localhost}:${TINYMEM_PORT:-3000}/session/$TINYMEM_SESSION/done" \
  -H "Authorization: Bearer $TINYMEM_TOKEN" >> /tmp/tinymem-stop-debug.log 2>&1

# Clean up
rm -f ".claude/.tinymem_session"
