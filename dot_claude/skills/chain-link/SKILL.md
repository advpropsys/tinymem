---
name: chain-link
description: Save a work checkpoint to preserve context for multi-session projects. Use when pausing work on a feature, completing a milestone, or before ending a session.
---

# Chain Link - Save Work Checkpoint

Save your current work context as a chain link for later retrieval.

## Usage

```
/chain-link [chain-name] [slug]
```

## Instructions

1. **Gather Context** - Analyze the current session:
   - What was accomplished
   - Key technical decisions made
   - Code changes with file paths
   - Unresolved issues or blockers
   - Next steps

2. **Generate Chain Content** - Structure as XML:

```xml
<chain-link>
  <summary>Brief overview of work done</summary>

  <completed>
    <item>Task or feature completed</item>
    <item>Another completed item</item>
  </completed>

  <code-changes>
    <file path="src/auth.rs">Added JWT validation middleware</file>
    <file path="src/routes.rs">New /login endpoint</file>
  </code-changes>

  <decisions>
    <decision reason="Performance">Used Redis for session storage</decision>
    <decision reason="Security">JWT tokens expire after 1 hour</decision>
  </decisions>

  <issues>
    <issue status="blocked">Rate limiting not implemented yet</issue>
    <issue status="investigation">Memory leak in connection pool</issue>
  </issues>

  <next-steps>
    <step priority="high">Implement rate limiting middleware</step>
    <step priority="medium">Add refresh token support</step>
  </next-steps>
</chain-link>
```

3. **Save via MCP** - Call `tinymem_chain_link`:
   - `session_id`: from TINYMEM_SESSION env
   - `chain_name`: provided or infer from task context
   - `slug`: provided or generate from summary (kebab-case)
   - `content`: XML structured content above

## Examples

User: `/chain-link auth-feature jwt-middleware`
→ Saves checkpoint with chain "auth-feature", slug "jwt-middleware"

User: `/chain-link`
→ Ask user for chain name, auto-generate slug from work summary
