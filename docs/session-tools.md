# Session Tools: Inter-Agent Communication

This document describes the session tools that enable persistent, asynchronous
agent-to-agent communication in Moltis.

## Overview

Moltis provides two patterns for agent coordination:

| Pattern | spawn_agent | sessions_send |
|---------|-------------|---------------|
| **Execution** | Synchronous (blocks until done) | Asynchronous (fire-and-forget or wait) |
| **Lifespan** | Ephemeral sub-agent | Persistent named session |
| **History** | None (fresh context) | Full conversation history |
| **Best for** | Single focused tasks | Ongoing coordination |
| **Depth limit** | 3 levels max | No nesting limit |

## Tools

### sessions_list

Discover active sessions accessible to the current agent.

```json
{
  "filter": "optional search string",
  "limit": 20
}
```

Returns:
```json
{
  "sessions": [
    {
      "key": "agent:alice:main",
      "label": "Alice's main session",
      "messageCount": 42,
      "createdAt": 1700000000000,
      "updatedAt": 1700001000000,
      "projectId": "proj_123",
      "model": "anthropic/claude-sonnet-4"
    }
  ],
  "count": 1
}
```

### sessions_history

Read messages from another session.

```json
{
  "key": "agent:researcher:main",
  "limit": 20,
  "offset": 0
}
```

Returns:
```json
{
  "key": "agent:researcher:main",
  "label": "Research session",
  "messages": [
    {"role": "user", "content": "Find auth patterns"},
    {"role": "assistant", "content": "Found 3 auth patterns..."}
  ],
  "totalMessages": 50,
  "hasMore": true
}
```

### sessions_send

Send a message to another session.

```json
{
  "key": "agent:coder:backend",
  "message": "Please implement JWT authentication",
  "wait_for_reply": true,
  "context": "researcher agent"
}
```

Returns (when `wait_for_reply: true`):
```json
{
  "key": "agent:coder:backend",
  "label": "Backend coding",
  "sent": true,
  "reply": "I've implemented JWT auth in src/auth/..."
}
```

Returns (when `wait_for_reply: false`):
```json
{
  "key": "agent:coder:backend",
  "label": "Backend coding",
  "sent": true,
  "message": "Message queued for delivery"
}
```

## Access Control

Session access is controlled by `SessionAccessPolicy`. Configure it in `moltis.toml`
within agent presets:

```toml
[agents.presets.coordinator]
identity.name = "orchestrator"
model = "anthropic/claude-sonnet-4-20250514"
tools.allow = ["sessions_list", "sessions_history", "sessions_send"]
# Full session access (default)
sessions.can_send = true

[agents.presets.worker]
identity.name = "worker"
tools.allow = ["sessions_list", "sessions_history"]
tools.deny = ["sessions_send"]
# Restricted access: only see sessions with "agent:worker:" prefix
sessions.key_prefix = "agent:worker:"
# Plus explicit access to shared coordination session
sessions.allowed_keys = ["shared:coordinator"]
sessions.can_send = false  # Read-only

[agents.presets.isolated]
# Completely isolated: no session visibility
sessions.key_prefix = "agent:isolated:"
sessions.can_send = false
sessions.cross_agent = false
```

### Policy Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `key_prefix` | string | none | Only see sessions matching this prefix |
| `allowed_keys` | list | `[]` | Additional session keys to allow access |
| `can_send` | bool | `true` | Whether `sessions_send` is permitted |
| `cross_agent` | bool | `false` | Access sessions from other agents |

### Internal Representation

The policy is represented internally as:

```rust
SessionAccessPolicy {
    key_prefix: Some("agent:myagent:".into()),
    allowed_keys: vec!["shared:global".into()],
    can_send: true,
    cross_agent: false,
}
```

### Default Behavior

- **No policy**: All sessions are accessible (for single-agent setups)
- **With prefix**: Only sessions matching the prefix are visible
- **Allowed keys**: Explicit exceptions to the prefix rule
- **can_send**: Controls whether `sessions_send` is permitted

## When to Use Each Pattern

### Use spawn_agent when:

- Task is well-defined and self-contained
- Sub-agent doesn't need prior conversation context
- You want cost optimization (cheaper models for sub-tasks)
- Task should complete before parent continues
- You need tool restrictions for the sub-task

```
User: "Add authentication to the API"

Main Agent (sonnet):
  └─ spawn_agent(preset="researcher", task="Find existing auth patterns")
     ↳ Sub-agent (haiku) runs, returns findings
  └─ spawn_agent(preset="coder", task="Implement JWT middleware")
     ↳ Sub-agent (sonnet) runs, returns code
```

### Use sessions_send when:

- Coordinating between long-running specialized agents
- Need to share context or continue existing conversations
- Asynchronous workflows (send and continue working)
- Multiple agents need to collaborate on same problem
- Agent needs to "check in" with another agent periodically

```
Research Session (ongoing):
  ↳ Building knowledge base about codebase patterns

Coding Session (ongoing):
  ↳ Implementing features, asks research session for patterns:
     sessions_send(key="research", message="What auth pattern does this project use?")
```

## Example: Multi-Agent Workflow

### Setup (moltis.toml)

```toml
[agents.presets.researcher]
identity.name = "scout"
identity.emoji = "🔍"
model = "anthropic/claude-haiku-3-5-20241022"
tools.allow = ["read_file", "glob", "grep", "sessions_list", "sessions_history"]
# No sessions_send - read-only coordinator

[agents.presets.coder]
identity.name = "forge"
identity.emoji = "⚡"
model = "anthropic/claude-sonnet-4-20250514"
tools.allow = ["read_file", "write_file", "exec", "sessions_send"]
# Can send findings to other sessions
```

### Coordinator Agent Workflow

```markdown
1. List available sessions:
   sessions_list({})
   → Found: "research", "frontend", "backend"

2. Check research progress:
   sessions_history(key="research", limit=5)
   → Recent messages show auth patterns found

3. Delegate implementation:
   sessions_send(
     key="backend",
     message="Implement JWT auth using the pattern from research",
     context="coordinator"
   )

4. Continue coordinating other work...
```

## Comparison with OpenClaw

This implementation follows the OpenClaw `sessions_*` pattern:

| OpenClaw | Moltis |
|----------|--------|
| `sessions_list` | `sessions_list` |
| `sessions_history` | `sessions_history` |
| `sessions_send` | `sessions_send` |

Key differences:
- Moltis uses SQLite-backed metadata (vs JSON file)
- Access control via `SessionAccessPolicy` struct
- Integrated with existing session infrastructure
- WebSocket events for real-time UI updates

## Implementation Details

### File Locations

- `crates/tools/src/sessions.rs` - Tool implementations
- `crates/gateway/src/server.rs` - Gateway integration (tool registration)
- `crates/sessions/src/metadata.rs` - Session metadata store
- `crates/sessions/src/store.rs` - Message storage (JSONL)

### Dependencies

The tools require:
- `SqliteSessionMetadata` for session discovery
- `SessionStore` for message history
- `SendToSessionFn` callback for message delivery (provided by gateway)

### Gateway Integration

The session tools are registered in `server.rs` alongside other tools. The
gateway provides:

1. **Tool registration**: All three tools (`sessions_list`, `sessions_history`,
   `sessions_send`) are registered in the tool registry
2. **Send callback**: Routes messages through the chat service with support
   for both async (fire-and-forget) and sync (wait for reply) modes
3. **WebSocket events**: Broadcasts `cross_session_send` events on the
   `sessions` channel for UI visibility

```rust
// Gateway registration (simplified from server.rs)
tool_registry.register(Box::new(SessionsListTool::new(session_metadata)));
tool_registry.register(Box::new(SessionsHistoryTool::new(session_store, session_metadata)));

let send_fn: SendToSessionFn = Arc::new(move |key, msg, wait| {
    let state = Arc::clone(&state);
    Box::pin(async move {
        // Broadcast event for cross-session visibility
        broadcast(&state, "sessions", json!({
            "event": "cross_session_send",
            "targetSession": key,
        }), BroadcastOpts::default()).await;

        let chat = state.chat().await;
        if wait {
            let result = chat.send_sync(params).await?;
            Ok(result["text"].as_str().unwrap_or("").to_string())
        } else {
            chat.send(params).await?;
            Ok(String::new())
        }
    })
});
tool_registry.register(Box::new(SessionsSendTool::new(metadata, send_fn)));
```

### WebSocket Events

The `sessions` channel broadcasts events for UI components:

```json
{
  "event": "cross_session_send",
  "targetSession": "agent:researcher:main",
  "messagePreview": "What auth pattern does this project use?",
  "waitForReply": true
}
```

### Next Steps

1. **UI Components**: Add session activity cards showing cross-session
   communication (see `docs/design/multi-agent-ui.md`)
2. **Access Policies**: Configure per-agent access policies in `moltis.toml`
3. **Agent Presets**: Add session tools to preset tool policies

Example preset configuration:

```toml
[agents.presets.coordinator]
tools.allow = ["sessions_list", "sessions_history", "sessions_send"]

[agents.presets.worker]
tools.allow = ["sessions_list", "sessions_history"]
tools.deny = ["sessions_send"]  # Read-only access
```
