---
name: chain-load
description: Load a chain to restore context from previous work. Use anytime when working on related topics - proactively search and suggest relevant chains.
---

# Chain Load - Load Work Context

Load chain links to restore context. Can be used anytime, not just at session start.

## Usage

```
/chain-load [chain-name]
/chain-load [search-query]
```

## Proactive Behavior

When working on a task, proactively check for relevant context:
1. Use `tinymem_chain_search` to find chains matching current work topic
2. Use `tinymem_search` to find individual memory segments
3. Suggest loading relevant chains or segments to the user

**Segments vs Chains:**
- **Segments** (memories): Individual insights via `tinymem_search` + `tinymem_get`
- **Chains**: Full workflow context via `tinymem_chain_load`

## Instructions

1. **Find Chain**:
   - If exact chain-name provided → use `tinymem_chain_load`
   - If query provided → use `tinymem_chain_search` for fuzzy matching
   - If nothing provided → use `tinymem_chain_list` to show all

2. **Load Chain Links** - Call `tinymem_chain_load`:
   - `chain_name`: the chain to load
   - `limit`: 5 (default, most recent links)

3. **Parse and Present Context** - Extract from XML:
   - Previous summaries
   - Decisions with rationale
   - Code changes history
   - Outstanding issues
   - Next steps

## Output Format

```
## Chain: [name] ([count] links)

### Latest: [slug] - [date]
<summary and next-steps from most recent link>

### Context
<key decisions and history from older links>

### Suggested Action
[Next step based on chain state]
```

## Examples

User: `/chain-load auth-feature`
→ Loads auth-feature chain

User: `/chain-load auth`
→ Searches chains matching "auth"

User working on auth code (proactive):
→ Agent: "Found chain 'auth-feature' with 3 links. Load context?"
