#!/bin/bash
# tinymem hooks installer
# Usage: ./install.sh /path/to/your/project

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SOURCE_DIR="$SCRIPT_DIR/dot_claude"

if [ -z "$1" ]; then
  echo "Usage: $0 <target-project-directory>"
  echo "Example: $0 /home/user/my-project"
  exit 1
fi

TARGET_DIR="$1"

if [ ! -d "$TARGET_DIR" ]; then
  echo "Error: Directory '$TARGET_DIR' does not exist"
  exit 1
fi

CLAUDE_DIR="$TARGET_DIR/.claude"
HOOKS_DIR="$CLAUDE_DIR/hooks"
SKILLS_DIR="$CLAUDE_DIR/skills"

# Create directories
mkdir -p "$HOOKS_DIR"
mkdir -p "$SKILLS_DIR"

# Copy hook scripts
echo "Installing hooks..."
cp "$SOURCE_DIR/hooks/tinymem-start.sh" "$HOOKS_DIR/"
cp "$SOURCE_DIR/hooks/tinymem-hook.sh" "$HOOKS_DIR/"
cp "$SOURCE_DIR/hooks/tinymem-stop.sh" "$HOOKS_DIR/"
cp "$SOURCE_DIR/hooks/tinymem-ask-intercept.sh" "$HOOKS_DIR/"
chmod +x "$HOOKS_DIR"/tinymem-*.sh

# Copy skills
echo "Installing skills..."
if [ -d "$SOURCE_DIR/skills" ]; then
  cp -r "$SOURCE_DIR/skills/"* "$SKILLS_DIR/" 2>/dev/null || true
fi

# Merge or create settings.json
SETTINGS_FILE="$CLAUDE_DIR/settings.json"

if [ -f "$SETTINGS_FILE" ] && [ -s "$SETTINGS_FILE" ]; then
  # File exists and is not empty
  echo "Merging into existing settings.json..."
  cp "$SETTINGS_FILE" "$SETTINGS_FILE.bak"

  if command -v jq &> /dev/null; then
    # Validate existing JSON first
    if ! jq empty "$SETTINGS_FILE.bak" 2>/dev/null; then
      echo "Warning: Existing settings.json is invalid JSON, replacing..."
      cp "$SOURCE_DIR/settings.json" "$SETTINGS_FILE"
    else
      # Read tinymem hooks
      tinymem_hooks=$(cat "$SOURCE_DIR/settings.json")

      # Merge: remove old tinymem hooks, add new ones (no duplicates)
      if jq --argjson new "$tinymem_hooks" '
        # Helper: filter out tinymem hooks safely
        def remove_tinymem: map(select((.hooks[0].command // "") | contains("tinymem") | not));
        # Update each hook type: remove existing tinymem, add new
        .hooks.SessionStart = ((.hooks.SessionStart // []) | remove_tinymem) + $new.hooks.SessionStart |
        .hooks.PreToolUse = ((.hooks.PreToolUse // []) | remove_tinymem) + $new.hooks.PreToolUse |
        .hooks.PostToolUse = ((.hooks.PostToolUse // []) | remove_tinymem) + $new.hooks.PostToolUse |
        # Remove any old tinymem Stop hooks (no longer used)
        .hooks.Stop = ((.hooks.Stop // []) | remove_tinymem)
      ' "$SETTINGS_FILE.bak" > "$SETTINGS_FILE.tmp" 2>/dev/null; then
        mv "$SETTINGS_FILE.tmp" "$SETTINGS_FILE"
        echo "Merged successfully (backup: settings.json.bak)"
      else
        echo "Error: Merge failed. Backup preserved at settings.json.bak"
        echo "Please manually merge $SOURCE_DIR/settings.json"
        exit 1
      fi
    fi
  else
    echo "Warning: jq not found, cannot merge automatically"
    echo "Please manually merge $SOURCE_DIR/settings.json into $SETTINGS_FILE"
    exit 1
  fi
else
  echo "Creating settings.json..."
  cp "$SOURCE_DIR/settings.json" "$SETTINGS_FILE"
fi

echo ""
echo "Hooks installed!"

# Install MCP server config (per-project .mcp.json)
TINYMEM_BIN="$SCRIPT_DIR/target/release/tinymem"
MCP_FILE="$TARGET_DIR/.mcp.json"

if [ ! -f "$TINYMEM_BIN" ]; then
  echo "Building release binary..."
  (cd "$SCRIPT_DIR" && cargo build --release --quiet 2>/dev/null)
fi

if [ -f "$TINYMEM_BIN" ]; then
  # Build tinymem MCP config
  TINYMEM_MCP=$(cat << EOF
{
  "mcpServers": {
    "tinymem": {
      "command": "$TINYMEM_BIN",
      "args": ["--mcp", "--port", "${TINYMEM_PORT:-3000}", "--token", "${TINYMEM_TOKEN:-}"]
    }
  }
}
EOF
)

  if [ -f "$MCP_FILE" ] && [ -s "$MCP_FILE" ]; then
    # Backup and merge
    cp "$MCP_FILE" "$MCP_FILE.bak"
    if jq empty "$MCP_FILE.bak" 2>/dev/null; then
      # Valid JSON - merge mcpServers
      if jq --argjson new "$TINYMEM_MCP" '.mcpServers.tinymem = $new.mcpServers.tinymem' "$MCP_FILE.bak" > "$MCP_FILE.tmp" 2>/dev/null; then
        mv "$MCP_FILE.tmp" "$MCP_FILE"
        echo "MCP config merged at $MCP_FILE (backup: .mcp.json.bak)"
      else
        echo "Error: MCP merge failed. Backup preserved at .mcp.json.bak"
        exit 1
      fi
    else
      echo "Error: Existing .mcp.json is invalid JSON. Backup preserved at .mcp.json.bak"
      exit 1
    fi
  else
    # Create new
    echo "$TINYMEM_MCP" > "$MCP_FILE"
    echo "MCP config created at $MCP_FILE"
  fi
fi

echo ""
echo "Installation complete!"
echo ""
echo "Next steps:"
echo "  1. Set environment (add to ~/.bashrc or ~/.zshrc):"
echo "     export TINYMEM_TOKEN=\"your-secret-token\""
echo "     export TINYMEM_PORT=\"3000\"  # if non-default"
echo ""
echo "  2. Re-run install.sh after setting env vars to update MCP config"
echo ""
echo "  3. Start tinymem server and restart Claude Code"
