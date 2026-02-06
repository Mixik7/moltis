# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Multi-Agent Coordination**: Full multi-agent system with sub-agent spawning,
  inter-agent communication, shared task tracking, and coordinator delegation

  - **Agent Presets**: Named presets in `moltis.toml` configure model, tools,
    system prompt, identity (name, creature, vibe, soul), memory, hooks, and
    delegate mode for sub-agents

  - **Session Access Policy**: `SessionAccessPolicy` wired into sub-agent
    spawning. Presets can restrict which sessions a sub-agent can read/write
    via prefix-based or explicit key rules. `ToolRegistry` gained `replace()`
    for in-place tool swapping

  - **Inter-Agent Session Tools**: Four new tools (`sessions_list`,
    `sessions_history`, `sessions_send`, `sessions_info`) for agent-to-agent
    communication over the existing session system

  - **Shared Task List**: `task_list` tool for cross-agent task tracking with
    create, list, get, update, and claim operations. Supports `blocked_by`
    dependencies, ownership, and JSON file persistence

  - **Persistent Agent Memory**: Sub-agents get `MEMORY.md` content (first 200
    lines) injected into their system prompt. Three scopes: user
    (`~/.moltis/agent-memory/<preset>/`), project (`.moltis/agent-memory/`),
    and local (`.moltis/agent-memory-local/`)

  - **Markdown Agent Definitions**: Define presets as `.md` files with YAML
    frontmatter in `~/.moltis/agents/` (user-global) or `.moltis/agents/`
    (project-local). Project overrides user; TOML takes precedence over
    markdown

  - **Shell Hooks for Tool Control**: Presets can define shell hooks that run
    on events (session start, tool calls). External commands receive JSON on
    stdin and signal via exit code (0 = continue, 1 = block, stdout JSON =
    modify payload)

  - **Delegate Mode**: `delegate_only = true` restricts agents to
    coordination-only tools (`spawn_agent`, `sessions_list`,
    `sessions_history`, `sessions_send`, `task_list`) with coordinator prompt
    injection

  - **Multi-Agent UI**: Web components for agent spawning, session list/history
    views, and real-time WebSocket event integration

- **QMD Backend Support**: Optional QMD (Query Memory Daemon) backend for hybrid search with BM25 + vector + LLM reranking
  - Gated behind `qmd` feature flag (enabled by default)
  - Web UI shows installation instructions and QMD status
  - Comparison table between built-in SQLite and QMD backends
- **Citations**: Configurable citation mode (on/off/auto) for memory search results
  - Auto mode includes citations when results span multiple files
- **Session Export**: Option to export session transcripts to memory for future reference
- **LLM Reranking**: Use LLM to rerank search results for improved relevance (requires QMD)
- **Memory Documentation**: Added `docs/src/memory.md` with comprehensive memory system documentation
- **Mobile PWA Support**: Install moltis as a Progressive Web App on iOS, Android, and desktop
  - Standalone mode with full-screen experience
  - Custom app icon (crab mascot)
  - Service worker for offline support and caching
  - Safe area support for notched devices

- **Push Notifications**: Receive alerts when the LLM responds
  - VAPID key generation and storage for Web Push API
  - Subscribe/unsubscribe toggle in Settings > Notifications
  - Subscription management UI showing device name, IP address, and date
  - Remove any subscription from any device
  - Real-time subscription updates via WebSocket
  - Client IP detection from X-Forwarded-For, X-Real-IP, CF-Connecting-IP headers
  - Notifications sent for both streaming and agent (tool-using) chat modes

- **Safari/iOS PWA Detection**: Show "Add to Dock" instructions when push notifications
  require PWA installation (Safari doesn't support push in browser mode)

### Changed

- Memory settings UI enhanced with backend comparison and feature explanations
- Added `memory.qmd.status` RPC method for checking QMD availability
- Extended `memory.config.get` to include `qmd_feature_enabled` flag

- Push notifications feature is now enabled by default in the CLI
- Extracted `SecuritySection`, `ApiKeysSection`, `PasskeysSection`,
  `PasswordSection` into separate Preact components from the settings page

- **TLS HTTP redirect port** now defaults to `gateway_port + 1` instead of
  the hardcoded port `18790`. This makes the Dockerfile simpler (both ports
  are adjacent) and avoids collisions when running multiple instances.
  Override via `[tls] http_redirect_port` in `moltis.toml` or the
  `MOLTIS_TLS__HTTP_REDIRECT_PORT` environment variable.

- **TLS certificates use `moltis.localhost` domain.** Auto-generated server
  certs now include `moltis.localhost`, `*.moltis.localhost`, `localhost`,
  `127.0.0.1`, and `::1` as SANs. Banner and redirect URLs use
  `https://moltis.localhost:<port>` when bound to loopback, so the cert
  matches the displayed URL. Existing certs are automatically regenerated
  on next startup.

- **Certificate validity uses dynamic dates.** Cert `notBefore`/`notAfter`
  are now computed from the current system time instead of being hardcoded.
  CA certs are valid for 10 years, server certs for 1 year from generation.

### Fixed

- Push notifications not sending when chat uses agent mode (run_with_tools)
- Missing space in Safari install instructions ("usingFile" → "using File")
- **WebSocket origin validation** now treats `.localhost` subdomains
  (e.g. `moltis.localhost`) as loopback equivalents per RFC 6761.

### Documentation

- Added `docs/design/multi-agent-architecture.md` — architecture design doc
- Added `docs/design/multi-agent-ui.md` — UI design doc
- Added `docs/src/agent-presets.md` — agent presets user guide
- Added `docs/src/session-tools.md` — session tools user guide
- Added mobile-pwa.md with PWA installation and push notification documentation
- Updated CLAUDE.md with cargo feature policy (features enabled by default)
