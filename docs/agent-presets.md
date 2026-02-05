# Agent Presets

Agent presets define reusable configurations for sub-agents spawned via the
`spawn_agent` tool. Each preset can specify a different model, tool access
policy, identity, and system prompt—enabling cost optimization and capability
restriction for delegated tasks.

## Quick Start

Add presets to your `moltis.toml`:

```toml
[agents.presets.researcher]
identity.name = "scout"
identity.emoji = "🔍"
model = "anthropic/claude-haiku-3-5-20241022"
tools.deny = ["exec", "write_file"]
system_prompt_suffix = "Focus on gathering information. Do not modify files."

[agents.presets.coder]
identity.name = "forge"
identity.emoji = "⚡"
model = "anthropic/claude-sonnet-4-20250514"
tools.deny = ["spawn_agent"]
max_iterations = 50
```

The LLM can then spawn sub-agents with these presets:

```json
{
  "tool": "spawn_agent",
  "params": {
    "task": "Find all authentication-related code in the codebase",
    "preset": "researcher"
  }
}
```

## Configuration Reference

### `[agents]` Section

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default` | string | none | Default preset name when none specified |
| `presets` | table | `{}` | Named preset definitions |

### `[agents.presets.<name>]` Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `identity.name` | string | none | Sub-agent's name in prompts |
| `identity.emoji` | string | none | Emoji identifier |
| `identity.creature` | string | none | What the agent "is" (e.g., "helpful owl") |
| `identity.vibe` | string | none | Personality style (e.g., "focused and efficient") |
| `identity.soul` | string | none | Freeform personality text |
| `model` | string | parent's model | LLM model ID (e.g., `anthropic/claude-haiku-3-5-20241022`) |
| `tools.allow` | list | `[]` (all) | Whitelist of allowed tools |
| `tools.deny` | list | `[]` | Blacklist of denied tools |
| `system_prompt_suffix` | string | none | Text appended to sub-agent's system prompt |
| `max_iterations` | int | 25 | Maximum agent loop iterations |
| `timeout_secs` | int | 600 | Wall-clock timeout in seconds |
| `sandbox.enabled` | bool | inherit | Override sandbox mode |
| `sandbox.image` | string | inherit | Override sandbox image |
| `sandbox.no_network` | bool | inherit | Override network isolation |

## Tool Policies

Tool policies control which tools a sub-agent can access:

### Allow List (Whitelist)

When `tools.allow` is non-empty, **only** those tools are available:

```toml
[agents.presets.reader]
tools.allow = ["read_file", "glob", "grep"]
# Sub-agent can ONLY use read_file, glob, grep
```

### Deny List (Blacklist)

When `tools.deny` is specified, those tools are removed:

```toml
[agents.presets.safe]
tools.deny = ["exec", "write_file"]
# Sub-agent can use everything EXCEPT exec and write_file
```

### Combined Policies

When both are specified, `allow` acts as a whitelist and `deny` further
restricts it:

```toml
[agents.presets.limited]
tools.allow = ["read_file", "write_file", "exec"]
tools.deny = ["exec"]
# Sub-agent can use read_file and write_file (exec is denied)
```

### Always Excluded

The `spawn_agent` tool is **always** excluded from sub-agents to prevent
infinite recursion, regardless of policy settings.

### Session Tools

The session tools enable inter-agent communication. Include them in presets
based on the agent's role:

| Tool | Purpose | Typical Use |
|------|---------|-------------|
| `sessions_list` | Discover available sessions | Coordinators, observers |
| `sessions_history` | Read messages from sessions | Coordinators, observers |
| `sessions_send` | Send messages to sessions | Coordinators only |

**Coordinator pattern** (full access):
```toml
tools.allow = ["sessions_list", "sessions_history", "sessions_send"]
```

**Observer pattern** (read-only):
```toml
tools.allow = ["sessions_list", "sessions_history"]
tools.deny = ["sessions_send"]
```

See [Session Tools](session-tools.md) for detailed documentation.

## System Prompt Construction

When a preset is used, the sub-agent's system prompt is built as follows:

1. **Identity preamble** (if preset has identity fields):
   ```
   You are scout, a helpful owl. Your style is focused and efficient.
   ```

2. **Base instruction**:
   ```
   Complete the task thoroughly and return a clear result.
   ```

3. **Task**:
   ```
   Task: Find all authentication-related code in the codebase
   ```

4. **Context** (if provided):
   ```
   Context: Focus on the src/auth directory
   ```

5. **Suffix** (if preset has `system_prompt_suffix`):
   ```
   Focus on gathering information. Do not modify files.
   ```

## Model Selection Priority

When spawning a sub-agent, the model is selected in this order:

1. **Explicit `model` parameter** in the spawn_agent call
2. **Preset's `model`** field
3. **Parent agent's model** (default)

This allows cost optimization—use expensive models for the main agent and
cheaper models for research/background tasks:

```toml
[agents.presets.background]
model = "anthropic/claude-haiku-3-5-20241022"  # ~10x cheaper
```

## Example Presets

### Researcher (Read-Only)

For information gathering without file modifications:

```toml
[agents.presets.researcher]
identity.name = "scout"
identity.emoji = "🔍"
identity.vibe = "thorough and methodical"
model = "anthropic/claude-haiku-3-5-20241022"
tools.allow = ["read_file", "glob", "grep", "web_search", "web_fetch"]
system_prompt_suffix = """
You are a research specialist. Your job is to find and report information.
Never modify files or execute commands—only search and analyze.
Return findings in a structured format with file paths and line numbers.
"""
```

### Coder (Implementation Focus)

For writing code without spawning more sub-agents:

```toml
[agents.presets.coder]
identity.name = "forge"
identity.emoji = "⚡"
identity.vibe = "efficient and precise"
model = "anthropic/claude-sonnet-4-20250514"
tools.deny = ["spawn_agent", "web_search", "web_fetch"]
max_iterations = 50
system_prompt_suffix = """
Focus on implementation. Write clean, tested code.
Don't search the web—use the information provided.
"""
```

### Reviewer (Analysis Only)

For code review without modifications:

```toml
[agents.presets.reviewer]
identity.name = "lens"
identity.emoji = "🔬"
identity.vibe = "meticulous code reviewer"
model = "anthropic/claude-sonnet-4-20250514"
tools.allow = ["read_file", "glob", "grep"]
system_prompt_suffix = """
Analyze code for:
- Bugs and logic errors
- Security vulnerabilities
- Performance issues
- Style and maintainability

Never modify files. Provide actionable feedback with specific line references.
"""
```

### Coordinator (Cross-Session Communication)

For orchestrating multiple agents via session tools:

```toml
[agents.presets.coordinator]
identity.name = "orchestrator"
identity.emoji = "🎯"
identity.vibe = "strategic and organized"
model = "anthropic/claude-sonnet-4-20250514"
tools.allow = [
  "sessions_list",
  "sessions_history",
  "sessions_send",
  "spawn_agent",
  "read_file",
  "glob",
  "grep"
]
system_prompt_suffix = """
You coordinate work across multiple agents. Use session tools to:
- List available sessions with sessions_list
- Check progress with sessions_history
- Delegate tasks with sessions_send

Prefer sessions_send for ongoing collaboration and spawn_agent for one-off tasks.
"""
```

### Worker (Read-Only Session Access)

For agents that can observe but not send to other sessions:

```toml
[agents.presets.worker]
identity.name = "worker"
identity.emoji = "⚙️"
model = "anthropic/claude-sonnet-4-20250514"
tools.allow = ["read_file", "write_file", "exec", "sessions_list", "sessions_history"]
tools.deny = ["sessions_send", "spawn_agent"]
system_prompt_suffix = """
Focus on your assigned task. You can view other sessions for context
but cannot send messages to them. Report results back to your caller.
"""
```

### Quick Task (Fast & Cheap)

For simple, quick operations:

```toml
[agents.presets.quick]
model = "anthropic/claude-haiku-3-5-20241022"
max_iterations = 10
timeout_secs = 60
```

## Spawn Agent Tool Reference

The `spawn_agent` tool accepts these parameters:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `task` | string | yes | Task description for the sub-agent |
| `context` | string | no | Additional context |
| `model` | string | no | Model ID override |
| `preset` | string | no | Preset name from config |

### Example Tool Calls

**With preset:**
```json
{
  "task": "Find all TODO comments in the codebase",
  "preset": "researcher"
}
```

**With preset and context:**
```json
{
  "task": "Review the authentication implementation",
  "preset": "reviewer",
  "context": "Focus on the JWT token handling in src/auth/"
}
```

**With preset and model override:**
```json
{
  "task": "Quick syntax check",
  "preset": "reviewer",
  "model": "anthropic/claude-haiku-3-5-20241022"
}
```

## Result Format

The spawn_agent tool returns:

```json
{
  "text": "Sub-agent's final response text",
  "iterations": 5,
  "tool_calls_made": 12,
  "model": "anthropic/claude-haiku-3-5-20241022",
  "preset": "researcher"
}
```

## Nesting Limits

Sub-agents have a maximum nesting depth of **3** to prevent infinite
recursion. The `spawn_agent` tool is automatically excluded from sub-agent
tool registries, so sub-agents cannot spawn their own sub-agents.

## Events

The gateway broadcasts WebSocket events for sub-agent lifecycle:

**Start:**
```json
{
  "event": "chat",
  "payload": {
    "state": "sub_agent_start",
    "task": "Find auth patterns",
    "model": "anthropic/claude-haiku-3-5-20241022",
    "depth": 1
  }
}
```

**End:**
```json
{
  "event": "chat",
  "payload": {
    "state": "sub_agent_end",
    "task": "Find auth patterns",
    "model": "anthropic/claude-haiku-3-5-20241022",
    "depth": 1,
    "iterations": 5,
    "toolCallsMade": 12
  }
}
```

## Best Practices

1. **Use cheaper models for research**: Haiku is ~10x cheaper than Sonnet and
   works well for information gathering.

2. **Restrict tools appropriately**: A researcher doesn't need `exec` or
   `write_file`. Fewer tools = faster responses and lower risk.

3. **Set reasonable timeouts**: Background tasks shouldn't run forever. Use
   `timeout_secs` to cap execution time.

4. **Keep prompts focused**: The `system_prompt_suffix` should be short and
   specific to the preset's purpose.

5. **Name presets clearly**: Use names that describe the role (`researcher`,
   `coder`, `reviewer`) not the task.
