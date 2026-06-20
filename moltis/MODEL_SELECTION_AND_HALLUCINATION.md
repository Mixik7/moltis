# Moltis — Model Selection & the Hallucination Root Cause (authoritative)

**Status:** authoritative / evidence-based. **Date:** 2026-06-20.
**Supersedes** the stale/incorrect claims in [`SETUP_GUIDE.md`](SETUP_GUIDE.md) ("LLM: Gemini 2.0 Flash",
"anti-hallucination guards = known fake data list") and the `SOUL.md` "Known Fake Data" blocklist.

This doc exists because earlier docs **disinformed**: they framed Moltis's hallucination as a prompt issue
and a missing-tools issue. Both are wrong. This is the verified truth, traced from production logs and a live
end-to-end test, so nobody chases the wrong cause again.

---

## 1. The root cause (verified)

**Moltis hallucinated channel/account data because the configured model was a weak tool-caller that returned
`tool_calls=0` — it answered from "knowledge" instead of calling the MCP tools that were available to it.**

Production-log evidence (`moltis_agents::runner`, 92 tools loaded, `native_tools=true`):

| Model | tool_calls | Result |
|---|---|---|
| `google/gemini-2.5-flash` | **0** | hallucinated (no tool called) |
| `google/gemma-4-31b-it` | **0** | hallucinated |
| `openai/gpt-5.1-chat` | **1** | worked (called the tool) |
| `z-ai/glm-4.7` (after fix, live test) | **1+** | **worked — called `list_accounts`, returned REAL data matching `/api/diagnostic`** |

Live verification (2026-06-20): with `z-ai/glm-4.7`, the prompt *"покажи мои аккаунты"* produced a real
`mcp__tg-kombain-production-a5d5__list_accounts` call returning the actual fleet (5 active, 6 frozen, the
known `*7759089107` no-proxy account) — i.e. **real tool data, not fabrication.** The fix is confirmed.

## 2. What is NOT the cause (corrections to prior disinformation)

- ❌ **NOT a "markdown tool_call fallback" (`prompt.rs:175`).** In production `native_tools=true` — tool schemas
  are sent via the native API (`runner.rs:781` → `provider.complete(messages, schemas_for_api)`), and the
  text fallback is appended only when `native_tools=false` (`prompt.rs:468`). The native path is wired
  correctly (`openai.rs` sends `tools`, parses `tool_calls`).
- ❌ **NOT "0 tools loaded."** Tools DO load: `MCP tools synced into tool registry tools=104` (76 TG-Kombain +
  28 n8n). Occasional SSE drops create brief windows (see §6), but the steady state is 104 tools present.
- ❌ **The `SOUL.md` "Known Fake Data" blocklist is NOT a fix.** It is a symptom-patch: the team saw specific
  fabricated @usernames and forbade those exact strings. A blocklist cannot make a weak model call tools — it
  only suppresses the specific fakes already seen; the model invents new ones. Keep the data-integrity
  *principles* in SOUL ("call tools, present only real data"), drop reliance on the hardcoded list.

## 3. How model selection works

- Provider is **OpenRouter** (`custom-openrouter-ai`), key + base in `provider_keys.json` on the volume
  (`/data/config/provider_keys.json`). ElectronHub was a second provider — **dead (free quota exhausted),
  removed 2026-06-20.**
- OpenRouter's **full live catalog (~300 models)** is fetched at runtime and shown in the picker (Web UI model
  cards + the `model` field in the TG bot). `provider_keys.json.models[]` is a *preferred/pin* list, not a
  filter. To hard-trim the picker: set `fetch_models=false` + an explicit `models=[...]` in the provider config
  (optional — not required to fix hallucination).
- **Active model = first prioritized registry entry, overridable per session** (`chat.rs::resolve_provider`).
  So the fix is to *pick/pin a strong model* — trimming the list is convenience, not the fix.

## 4. Curated model guidance (live OpenRouter prices, June 2026, $/1M in·out)

**Hard rule:** the model MUST reliably emit native tool calls. Russian quality + cost are secondary.

### ✅ KEEP — by use case

**Workhorse (bulk chats / comments / invites / research — cheap + good Russian + calls tools):**
`anthropic/claude-haiku-4.5` (1·5, best Russian) · `z-ai/glm-4.7` (0.40·1.75, agentic, cheap — **current
default**) · `z-ai/glm-4.7-flash` (0.06·0.40, cheapest) · `qwen/qwen-plus` (0.26·0.78, strong multilingual) ·
`openai/gpt-5-mini` (0.25·2.00) · `deepseek/deepseek-v3.2` (0.23·0.34, fallback — Russian can drift to English)

**Mid (agent brain — balance):**
`openai/gpt-5.1-chat` (1.25·10, proven in our logs) · `x-ai/grok-4.3` (1.25·2.50, 1M ctx) ·
`z-ai/glm-5` (0.60·1.92) · `qwen/qwen3.7-max` (1.25·3.75) · `anthropic/claude-sonnet-4.6` (3·15, 1M ctx, top Russian)

**Heavy (rare — self-rewrite, multi-hour, big context):**
`anthropic/claude-opus-4.8` (5·25, 1M) · `openai/gpt-5.5` (5·30, 1.05M) ·
`google/gemini-3.1-pro-preview` (2·12, 1M — Pro is fine, Flash is NOT) · `z-ai/glm-5.2` (1.20·4.10, 1M) ·
`x-ai/grok-4.20` (1.25·2.50, **2M ctx**) · `openai/gpt-5.1-codex` (1.25·10, code self-rewrite)

### 🚫 AVOID (proven weak tool-callers → hallucinate)
`google/gemini-2.5-flash`, any `google/gemma-*`, any `*-flash-lite`, anything `:free`, `gpt-3.5`, and models
under ~14B. These return `tool_calls=0` and fabricate. Confirmed by our logs AND Google's own dev forums
(Gemini function-calling "ANY mode sometimes returns text", Flash-Lite spurious `UNEXPECTED_TOOL_CALL`).

> Open-weight models (GLM/Qwen/DeepSeek) are served by multiple OpenRouter providers; tool-call reliability
> varies by provider. Enable OpenRouter provider routing / verify before high-volume use.

## 5. Billing — OpenRouter credits required

If you see `HTTP 402: requires more credits` (incl. "Auto-compact failed"), OpenRouter is out of credits.
Top up at https://openrouter.ai/settings/credits. This is billing, not a bug — even a correct model + correct
tool calls stop when credits run out mid-task.

## 6. MCP connection facts (verified)

- **TG-Kombain MCP** (`/mcp/?token=...`): healthy, serves **76 tools** (verified via `/api/mcp-status`:
  `mounted:true, error:null, tools:76`). Occasionally the SSE connection drops (`state=dead`); the first
  auto-reconnect attempt often fails (`MCP initialize request failed`) and the second succeeds (~30-90s window
  without the 76 tools). Health monitor only restarts `dead`/`stopped` servers — a "running but 0 tools" state
  is a known blind spot (see §7).
- **N8N MCP** (`n8n-production-fc90`): **BROKEN.** `MCP OAuth token refresh failed ... 400` every 30s — ~89% of
  Moltis log volume. The "authenticated / running" claim in SETUP_GUIDE Step 2 is stale. Fix or disable the
  n8n MCP OAuth registration; until then its 28 tools are intermittently unavailable and the logs are flooded.

## 7. The permanent fix (recommended, code — not prompt)

Model selection fixes hallucination *today*, but any weak model (or an OpenRouter provider that ignores
`tools`) can regress it. The durable fix is a **grounding gate** in the runner: when a data-requesting turn
finishes with `tool_calls=0`, do NOT return the model's text as data — force a tool call or return an explicit
"data unavailable, retry". That converts a silent fabrication into an honest failure regardless of model.
This is the proper, enforced version of the SOUL "don't hallucinate" plea. Tracked as a `moltis-fork` change
(`crates/agents/src/runner.rs` around the `tool_calls.is_empty()` return path, `runner.rs:995`).

## 8. How to verify (log greps)

```bash
railway link -p 5fdc9a7b-d7d7-4d25-8ecb-afc5cb5e6311 -e 92dfc0fa-5749-4c63-a204-c48544b6d34e -s 2052c2b8-3f69-4db4-8a5d-5b81e2ace13e
# did the model call tools? (filter out the n8n-oauth noise)
railway logs -n 4000 | grep -iE "agent loop|tool_calls=" | grep -v "OAuth token refresh"
# server side: is TG-Kombain serving 76 tools?  GET /api/mcp-status (admin Bearer)
```
A data request with `tool_calls=0` = weak model → switch per §4. `tool_calls=1+` with real data = healthy.
