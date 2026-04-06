# Moltis Atlas — Parsing Manager Setup Guide

## Current Status (2026-03-21, updated S83)

### DONE (verified via API + DM test + Telegram)
- [x] Step 1: TG-Kombain MCP — 76 tools (16 modules), state: running
- [x] Step 2: N8N MCP — 3 meta-tools, state: running, authenticated
- [x] Step 3: SOUL.md + parsing-manager skill — configured via Web UI
- [x] Step 4: Telegram @MOLTIS_TOP_BOT — channel configured, responds to DM
- [x] Step 5: Verified — all 3 bots respond correctly

### Voice & AI Stack (Phase 6.5)
- [x] **LLM**: Gemini 2.0 Flash via OpenRouter (primary)
- [x] **TTS**: OpenAI gpt-4o-mini-tts (voice: nova)
- [x] **STT**: Groq Whisper (whisper-large-v3-turbo, language: ru)
- [x] **Web Search**: Brave Search API (BRAVE_API_KEY)
- [x] **Fork**: github.com/Mixik7/moltis — patched base_url support for OpenAI TTS
- [x] **Deploy**: GitHub repo (not Docker image), railway.json with sh -c wrapper

---

## Architecture: Secretary + 2 Systems

```
                     Оператор (Telegram)
                          |
            +-------------+-------------+
            |             |             |
            v             v             v
     @IDEA_TOP_BOT  @My_Asst_c2_bot  @MOLTIS_TOP_BOT
     ══════════════  ═══════════════  ═══════════════
      СЕКРЕТАРЬ      КОНТЕНТ-ЗАВОД     ПАРСИНГ
      OpenClaw v3    N8N WF 01→05     Moltis Atlas ← YOU
      Gemini 3 Flash Preview  Claude Sonnet 4   Rust v0.9.0
      (polling)      (webhook)         (polling)
```

Moltis = **парсинг-менеджер**. Управляет TG-Kombain через 76 прямых MCP tools (16 modules). Deep research и аналитика. Работает автономно или по запросу секретаря.

---

## Step 1: TG-Kombain MCP Server (DONE)

Moltis connects directly to TG-Kombain MCP (not via proxy):

| Field | Value |
|-------|-------|
| Name | `tg-kombain-production-a5d5` |
| URL | `https://tg-kombain-production-a5d5.up.railway.app/mcp/` |
| Transport | Streamable HTTP (SSE fallback) |
| Tools | 76 (16 modules) |

**Note:** Trailing slash in URL is required. Auth REQUIRED on Railway: Bearer `MCP_ACCESS_TOKEN` or `?token=` query param. Without `MCP_ACCESS_TOKEN` env var = dev mode (no auth).

### 76 MCP Tools by Category (16 modules)

**System**: `system_health`, `system_stats`
**Parsing**: `parse_channel`, `parse_audience`, `get_parsed_users`, `get_parsed_channels`, `task_status`
**Analytics**: `get_audience_insights`, `get_kpi_report`, `get_quality_snapshot`
**Content**: `post_to_channel`, `create_scheduled_post`, `get_content_calendar`
**Engagement**: `send_reactions`, `send_ai_comments`
**Outreach**: `invite_users`, `send_direct_messages`
**Accounts**: `list_accounts`, `get_trust_score`, `start_warmup`, `stop_warmup`
**Ads**: `collect_ad_candidates`, `get_best_ad_channels`, `create_ad_campaign`
**PR**: `search_mutual_pr`, `propose_mutual_pr`
**Viral**: `create_viral_campaign`, `start_viral_campaign`, `get_campaign_status`
**Other**: remaining utility tools

---

## Step 2: N8N MCP Server (DONE)

N8N provides instance-level MCP with 3 meta-tools:

| Field | Value |
|-------|-------|
| Name | `n8n-production-fc90` |
| URL | `https://n8n-production-fc90.up.railway.app/mcp-server/http` |
| Auth | MCP Access Token (configured in N8N UI: Settings > MCP Access) |
| Tools | 3 (search_workflows, get_workflow_details, execute_workflow) |

**Available workflows** (5 with `availableInMCP: true`):
- WF 26 "Agent Dispatcher" — route tasks to any agent
- WF 27 "Result Collector" — receive agent results
- WF 28 "Analytics" — daily cross-agent KPI report
- WF 29 "Strategy" — collaborative content strategy
- WF 30 "Feedback Loop" — automated pipeline feedback

**IMPORTANT:** N8N MCP does NOT use API JWT. It uses OAuth2 or MCP Access Token (Settings > MCP Access in N8N Web UI).

---

## Step 3: Update SOUL.md (DONE — Web UI)

In Moltis Web UI → Settings → Identity → SOUL.md textarea, paste the contents of:

**Canonical file:** [`moltis-fork/SOUL.md.updated`](../../moltis-fork/SOUL.md.updated)

Key features of the current SOUL.md (S83, 2026-03-21):
- 76 MCP tools (16 modules) from TG-Kombain — all modules listed with individual tools
- 3 N8N meta-tools (search_workflows, get_workflow_details, execute_workflow)
- Anti-hallucination guards (known fake data list, data integrity rules)
- Parse -> Fetch mandatory workflow (5-step chain)
- Cron usage rules (session targets, forbidden patterns)
- Task lifecycle management (duplicate prevention)
- MCP error recovery protocol
- Result quality and server-side filtering rules

---

## Step 3b: Create Parsing Manager Skill (DONE — Web UI)

In Moltis Web UI → Skills → create (or update existing `agent-ecosystem`) personal skill:

**Name**: `parsing-manager`
**Description**: Quick actions for Telegram channel parsing, audience analysis, and growth automation

```markdown
---
name: parsing-manager
description: Quick actions for parsing, audience analysis, and growth automation via 76 TG-Kombain MCP tools
enabled: true
---

# Parsing Manager Skill

## Quick Actions

### Morning Check (run daily)
1. `system_health` — verify platform is operational
2. `list_accounts` — check all accounts
3. For each account: `get_trust_score` — flag any below threshold
4. `get_kpi_report` — pipeline metrics
5. Compile: Russian summary table, flag issues, recommend actions

### Parse New Channel
1. `parse_channel` with target channel URL/username
2. Monitor: `task_status` until complete
3. `get_parsed_users` — retrieve user list
4. `get_parsed_channels` — channel metadata
5. `get_audience_insights` — demographics and behavior analysis
6. Report: channel size, engagement rate, audience overlap, growth potential

### Audience Growth Campaign
1. `get_audience_insights` on target channel — understand current audience
2. `collect_ad_candidates` — find channels with matching audience
3. `get_best_ad_channels` — rank by efficiency
4. Choose strategy:
   - Ads: propose budget and placement
   - PR: `search_mutual_pr` → `propose_mutual_pr`
   - Viral: `create_viral_campaign` → `start_viral_campaign`
   - Direct: `invite_users` (respect rate limits!)

### Account Health Recovery
1. `list_accounts` — find unhealthy accounts
2. `get_trust_score` for each — identify issues
3. If trust low: `start_warmup` with conservative mode
4. If trust critical: recommend pausing all automation
5. Monitor: re-check trust scores after 24h

### Delegate to Content Team
When the operator asks about publishing, videos, or content creation:
1. Use N8N `execute_workflow` with WF 26 "Agent Dispatcher"
2. Input: `{"from_agent": "moltis", "to_agent": "n8n", "action": "delegate", "payload": {"message": "..."}}`
3. This is NOT your area — route to the content system
```

---

## Step 4: Telegram Bot @MOLTIS_TOP_BOT (DONE)

Already configured and verified. Channel: 1, state: active.
Responds to DM: "Я — Atlas, автономный менеджер..."

---

## Step 5: Verify (DONE)

Run these checks in Moltis Web UI chat:

1. **MCP tools check**: "Покажи список всех MCP tools" → should show 76 TG-Kombain + 3 N8N = 79 total
2. **System health**: "Проверь здоровье TG-Kombain" → should use `system_health`
3. **Account check**: "Покажи все аккаунты и trust score" → should use `list_accounts` + `get_trust_score`
4. **Parsing**: "Спарси канал @test_channel" → should use `parse_channel`
5. **N8N meta-tools**: "Найди доступные N8N воркфлоу" → should use `search_workflows`
6. **Delegation**: "Делегируй тестовое сообщение секретарю" → should use WF 26
7. **Telegram**: DM @MOLTIS_TOP_BOT → should respond

---

## API Reference

### Agent Bridge (TG-Kombain)
- `POST /api/agent/dispatch` — dispatch AgentMessage
- `POST /api/agent/result` — receive result
- `GET /api/agent/tasks` — list tasks
- `GET /api/agent/tasks/{id}` — task status
- Auth: Bearer API_SECRET_KEY

### MCP Direct (TG-Kombain)
- `POST /mcp/` — Streamable HTTP (trailing slash required!)
- Protocol: initialize → Mcp-Session-Id → tools/list → tool/call
- 76 tools (16 modules), auth REQUIRED on Railway (Bearer MCP_ACCESS_TOKEN or ?token=)

### N8N MCP
- `POST /mcp-server/http` — instance-level MCP
- 3 tools: search_workflows, get_workflow_details, execute_workflow
- Auth: MCP Access Token (Settings > MCP Access)

---

## Troubleshooting

| Проблема | Решение |
|----------|---------|
| MCP tools не видны | Moltis Web UI → MCP → check connection state. Reload if "error" |
| SSE session expired | Moltis auto-reconnects; if stuck, manually reload MCP connection |
| N8N MCP auth fail | Проверить MCP Access Token в N8N UI (Settings > MCP Access) — NOT API JWT |
| Trailing slash | URL must end with `/mcp/` (with slash) — without it = 307 redirect loop |
| Tools show 0 | Stateful protocol: initialize must complete before tools/list works |
