// ── Channels page (Preact + HTM + Signals) ──────────────────

import { signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { registerPage } from "./router.js";
import { connected, models as modelsSig } from "./signals.js";
import * as S from "./state.js";
import { ConfirmDialog, Modal, ModelSelect, requestConfirm, showToast } from "./ui.js";

var channels = signal([]);

export function prefetchChannels() {
	sendRpc("channels.status", {}).then((res) => {
		if (res?.ok) {
			var ch = res.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
		}
	});
}
var senders = signal([]);
var activeTab = signal("channels");
var showAddModal = signal(false);
var editingChannel = signal(null);
var sendersAccount = signal("");

function loadChannels() {
	sendRpc("channels.status", {}).then((res) => {
		if (res?.ok) {
			var ch = res.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
			updateNavCount("channels", ch.length);
		}
	});
}

function loadSenders() {
	var accountId = sendersAccount.value;
	if (!accountId) {
		senders.value = [];
		return;
	}
	sendRpc("channels.senders.list", { account_id: accountId }).then((res) => {
		if (res?.ok) senders.value = res.payload?.senders || [];
	});
}

// ── Channel icons (inline SVG via htm) ───────────────────────
function TelegramIcon() {
	return html`<svg width="16" height="16" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" stroke-width="1.5">
    <path d="M22 2L11 13M22 2l-7 20-4-9-9-4 20-7z" />
  </svg>`;
}

function WhatsAppIcon() {
	return html`<svg width="16" height="16" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" stroke-width="1.5">
    <path d="M3 21l1.65-3.8a9 9 0 1 1 3.4 2.9L3 21" />
    <path d="M9 10a.5.5 0 0 0 1 0V9a.5.5 0 0 0-1 0v1Zm0 0a5 5 0 0 0 5 5m0 0a.5.5 0 0 0 0-1h-1a.5.5 0 0 0 0 1h1Z" />
  </svg>`;
}

function ChannelIcon({ type }) {
	if (type === "whatsapp") return html`<${WhatsAppIcon} />`;
	return html`<${TelegramIcon} />`;
}

// ── Channel card ─────────────────────────────────────────────
function ChannelCard(props) {
	var ch = props.channel;
	var channelType = ch.type || "telegram";

	function onRemove() {
		requestConfirm(`Remove ${ch.name || ch.account_id}?`).then((yes) => {
			if (!yes) return;
			sendRpc("channels.remove", { account_id: ch.account_id, type: channelType }).then((r) => {
				if (r?.ok) loadChannels();
			});
		});
	}

	var statusClass = ch.status === "connected" ? "configured" : "oauth";
	var sessionLine = "";
	if (ch.sessions && ch.sessions.length > 0) {
		var active = ch.sessions.filter((s) => s.active);
		sessionLine =
			active.length > 0
				? active.map((s) => `${s.label || s.key} (${s.messageCount} msgs)`).join(", ")
				: "No active session";
	}

	var displayName = ch.name || ch.account_id || (channelType === "whatsapp" ? "WhatsApp" : "Telegram");

	return html`<div class="provider-card" style="padding:12px 14px;border-radius:8px;margin-bottom:8px;">
    <div style="display:flex;align-items:center;gap:10px;">
      <span style="display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;border-radius:6px;background:var(--surface2);">
        <${ChannelIcon} type=${channelType} />
      </span>
      <div style="display:flex;flex-direction:column;gap:2px;">
        <span class="text-sm text-[var(--text-strong)]">${displayName}</span>
        ${ch.details && html`<span class="text-xs text-[var(--muted)]">${ch.details}</span>`}
        ${sessionLine && html`<span class="text-xs text-[var(--muted)]">${sessionLine}</span>`}
      </div>
      <span class="provider-item-badge ${statusClass}">${ch.status || "unknown"}</span>
    </div>
    <div class="flex gap-2">
      <button class="provider-btn provider-btn-sm provider-btn-secondary" title="Edit ${ch.account_id || "channel"}"
        onClick=${() => {
					editingChannel.value = ch;
				}}>Edit</button>
      <button class="provider-btn provider-btn-sm provider-btn-danger" title="Remove ${ch.account_id || "channel"}"
        onClick=${onRemove}>Remove</button>
    </div>
  </div>`;
}

// ── Channels tab ─────────────────────────────────────────────
function ChannelsTab() {
	if (channels.value.length === 0) {
		return html`<div style="text-align:center;padding:40px 0;">
      <div class="text-sm text-[var(--muted)]" style="margin-bottom:12px;">No messaging channels connected.</div>
      <div class="text-xs text-[var(--muted)]">Click "+ Add Channel" to connect a Telegram bot or WhatsApp Business account.</div>
    </div>`;
	}
	return html`${channels.value.map((ch) => html`<${ChannelCard} key=${ch.account_id} channel=${ch} />`)}`;
}

// ── Senders tab ──────────────────────────────────────────────
function SendersTab() {
	useEffect(() => {
		if (channels.value.length > 0 && !sendersAccount.value) {
			sendersAccount.value = channels.value[0].account_id;
		}
	}, [channels.value]);

	useEffect(() => {
		loadSenders();
	}, [sendersAccount.value]);

	if (channels.value.length === 0) {
		return html`<div class="text-sm text-[var(--muted)]">No channels configured.</div>`;
	}

	function onAction(identifier, action) {
		var rpc = action === "approve" ? "channels.senders.approve" : "channels.senders.deny";
		sendRpc(rpc, {
			account_id: sendersAccount.value,
			identifier: identifier,
		}).then(() => {
			loadSenders();
			loadChannels();
		});
	}

	return html`<div>
    <div style="margin-bottom:12px;">
      <label class="text-xs text-[var(--muted)]" style="margin-right:6px;">Account:</label>
      <select style="background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:4px 8px;font-size:12px;"
        value=${sendersAccount.value} onChange=${(e) => {
					sendersAccount.value = e.target.value;
				}}>
        ${channels.value.map(
					(ch) => html`<option key=${ch.account_id} value=${ch.account_id}>${ch.name || ch.account_id}</option>`,
				)}
      </select>
    </div>
    ${senders.value.length === 0 && html`<div class="text-sm text-[var(--muted)] senders-empty">No messages received yet for this account.</div>`}
    ${
			senders.value.length > 0 &&
			html`<table class="senders-table">
      <thead><tr>
        <th class="senders-th">Sender</th><th class="senders-th">Username</th>
        <th class="senders-th">Messages</th><th class="senders-th">Last Seen</th>
        <th class="senders-th">Status</th><th class="senders-th">Action</th>
      </tr></thead>
      <tbody>
        ${senders.value.map((s) => {
					var identifier = s.username || s.peer_id;
					var lastSeenMs = s.last_seen ? s.last_seen * 1000 : 0;
					return html`<tr key=${s.peer_id}>
            <td class="senders-td">${s.sender_name || s.peer_id}</td>
            <td class="senders-td" style="color:var(--muted);">${s.username ? `@${s.username}` : "\u2014"}</td>
            <td class="senders-td">${s.message_count}</td>
            <td class="senders-td" style="color:var(--muted);font-size:12px;">${lastSeenMs ? html`<time data-epoch-ms="${lastSeenMs}">${new Date(lastSeenMs).toISOString()}</time>` : "\u2014"}</td>
            <td class="senders-td">
              ${
								s.otp_pending
									? html`<span class="provider-item-badge cursor-pointer select-none" style="background:var(--warning-bg, #fef3c7);color:var(--warning-text, #92400e);" onClick=${() => {
											navigator.clipboard.writeText(s.otp_pending.code).then(() => showToast("OTP code copied"));
										}}>OTP: <code class="text-xs">${s.otp_pending.code}</code></span>`
									: html`<span class="provider-item-badge ${s.allowed ? "configured" : "oauth"}">${s.allowed ? "Allowed" : "Denied"}</span>`
							}
            </td>
            <td class="senders-td">
              ${
								s.allowed
									? html`<button class="provider-btn provider-btn-sm provider-btn-danger" onClick=${() => onAction(identifier, "deny")}>Deny</button>`
									: html`<button class="provider-btn provider-btn-sm" onClick=${() => onAction(identifier, "approve")}>Approve</button>`
							}
            </td>
          </tr>`;
				})}
      </tbody>
    </table>`
		}
  </div>`;
}

// ── Tag-style allowlist input ────────────────────────────────
function AllowlistInput({ value, onChange }) {
	var input = useSignal("");

	function addTag(raw) {
		var tag = raw.trim().replace(/^@/, "");
		if (tag && !value.includes(tag)) onChange([...value, tag]);
		input.value = "";
	}

	function removeTag(tag) {
		onChange(value.filter((t) => t !== tag));
	}

	function onKeyDown(e) {
		if (e.key === "Enter" || e.key === ",") {
			e.preventDefault();
			if (input.value.trim()) addTag(input.value);
		} else if (e.key === "Backspace" && !input.value && value.length > 0) {
			onChange(value.slice(0, -1));
		}
	}

	return html`<div class="flex flex-wrap items-center gap-1.5 rounded border border-[var(--border)] bg-[var(--surface2)] px-2 py-1.5"
    style="min-height:38px;cursor:text;"
    onClick=${(e) => e.currentTarget.querySelector("input")?.focus()}>
    ${value.map(
			(tag) => html`<span key=${tag}
        class="inline-flex items-center gap-1 rounded-full bg-[var(--accent)]/10 px-2 py-0.5 text-xs text-[var(--accent)]">
        ${tag}
        <button type="button" class="inline-flex items-center text-[var(--muted)] hover:text-[var(--accent)]"
          style="line-height:1;font-size:14px;padding:0;background:none;border:none;cursor:pointer;"
          onClick=${(e) => {
						e.stopPropagation();
						removeTag(tag);
					}}>\u00d7</button>
      </span>`,
		)}
    <input type="text" value=${input.value}
      onInput=${(e) => {
				input.value = e.target.value;
			}}
      onKeyDown=${onKeyDown}
      placeholder=${value.length === 0 ? "Type a username and press Enter" : ""}
      class="flex-1 bg-transparent text-[var(--text)] text-sm outline-none border-none"
      style="min-width:80px;padding:2px 0;font-family:var(--font-body);" />
  </div>`;
}

// ── Add channel modal ────────────────────────────────────────
function AddChannelModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var channelType = useSignal("telegram");
	var allowlistItems = useSignal([]);

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = form.querySelector("[data-field=accountId]").value.trim();
		var type = channelType.value;

		if (!accountId) {
			error.value = type === "whatsapp" ? "Account ID is required." : "Bot username is required.";
			return;
		}

		var addConfig = {};

		if (type === "telegram") {
			var token = form.querySelector("[data-field=token]").value.trim();
			if (!token) {
				error.value = "Bot token is required.";
				return;
			}
			addConfig.token = token;
			addConfig.mention_mode = form.querySelector("[data-field=mentionMode]").value;
		} else if (type === "whatsapp") {
			var phoneNumberId = form.querySelector("[data-field=phoneNumberId]").value.trim();
			var accessToken = form.querySelector("[data-field=accessToken]").value.trim();
			var appSecret = form.querySelector("[data-field=appSecret]").value.trim();
			var verifyToken = form.querySelector("[data-field=verifyToken]").value.trim();
			if (!phoneNumberId) {
				error.value = "Phone Number ID is required.";
				return;
			}
			if (!accessToken) {
				error.value = "Access Token is required.";
				return;
			}
			if (!appSecret) {
				error.value = "App Secret is required.";
				return;
			}
			if (!verifyToken) {
				error.value = "Verify Token is required.";
				return;
			}
			addConfig.phone_number_id = phoneNumberId;
			addConfig.access_token = accessToken;
			addConfig.app_secret = appSecret;
			addConfig.verify_token = verifyToken;
		}
		addConfig.dm_policy = form.querySelector("[data-field=dmPolicy]").value;
		addConfig.allowlist = allowlistItems.value;

		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}

		error.value = "";
		saving.value = true;

		sendRpc("channels.add", {
			type: type,
			account_id: accountId,
			config: addConfig,
		}).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddModal.value = false;
				addModel.value = "";
				channelType.value = "telegram";
				allowlistItems.value = [];
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	var selectStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;cursor:pointer;";
	var inputStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;";

	var type = channelType.value;
	var title = type === "whatsapp" ? "Add WhatsApp Business" : "Add Telegram Bot";

	return html`<${Modal} show=${showAddModal.value} onClose=${() => {
		showAddModal.value = false;
		channelType.value = "telegram";
	}} title=${title}>
    <div class="channel-form">
      <label class="text-xs text-[var(--muted)]">Channel Type</label>
      <select data-field="channelType" style=${selectStyle} value=${type}
        onChange=${(e) => {
					channelType.value = e.target.value;
				}}>
        <option value="telegram">Telegram</option>
        <option value="whatsapp">WhatsApp Business</option>
      </select>

      ${
				type === "telegram" &&
				html`
        <div class="channel-card">
          <span class="text-xs font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
          <div class="text-xs text-[var(--muted)] channel-help">1. Open <a href="https://t.me/BotFather" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">@BotFather</a> in Telegram</div>
          <div class="text-xs text-[var(--muted)]">2. Send /newbot and follow the prompts to choose a name and username</div>
          <div class="text-xs text-[var(--muted)]">3. Copy the bot token (looks like 123456:ABC-DEF...) and paste it below</div>
          <div class="text-xs text-[var(--muted)] channel-help" style="margin-top:2px;">See the <a href="https://core.telegram.org/bots/tutorial" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">Telegram Bot Tutorial</a> for more details.</div>
        </div>
        <label class="text-xs text-[var(--muted)]">Bot username</label>
        <input data-field="accountId" type="text" placeholder="e.g. my_assistant_bot" style=${inputStyle} />
        <label class="text-xs text-[var(--muted)]">Bot Token (from @BotFather)</label>
        <input data-field="token" type="password" placeholder="123456:ABC-DEF..." style=${inputStyle} />
      `
			}

      ${
				type === "whatsapp" &&
				html`
        <div class="channel-card">
          <span class="text-xs font-medium text-[var(--text-strong)]">How to set up WhatsApp Business API</span>
          <div class="text-xs text-[var(--muted)] channel-help">1. Create a Meta Business account and set up a WhatsApp Business app</div>
          <div class="text-xs text-[var(--muted)]">2. Get your Phone Number ID, Access Token, and App Secret from the Meta Developer Console</div>
          <div class="text-xs text-[var(--muted)]">3. Configure a webhook URL pointing to: <code class="text-[var(--accent)]">/api/webhooks/whatsapp/[account_id]</code></div>
          <div class="text-xs text-[var(--muted)] channel-help" style="margin-top:2px;">See the <a href="https://developers.facebook.com/docs/whatsapp/cloud-api/get-started" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">WhatsApp Cloud API Guide</a> for details.</div>
        </div>
        <label class="text-xs text-[var(--muted)]">Account ID (unique identifier for this channel)</label>
        <input data-field="accountId" type="text" placeholder="e.g. my_business" style=${inputStyle} />
        <label class="text-xs text-[var(--muted)]">Phone Number ID (from Meta Business Suite)</label>
        <input data-field="phoneNumberId" type="text" placeholder="e.g. 123456789012345" style=${inputStyle} />
        <label class="text-xs text-[var(--muted)]">Access Token</label>
        <input data-field="accessToken" type="password" placeholder="EAAxxxx..." style=${inputStyle} />
        <label class="text-xs text-[var(--muted)]">App Secret (for webhook verification)</label>
        <input data-field="appSecret" type="password" placeholder="App secret from Meta Developer Console" style=${inputStyle} />
        <label class="text-xs text-[var(--muted)]">Verify Token (you choose this, use it when configuring the webhook)</label>
        <input data-field="verifyToken" type="text" placeholder="e.g. my_verify_token_123" style=${inputStyle} />
      `
			}

      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${selectStyle}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>

      ${
				type === "telegram" &&
				html`
        <label class="text-xs text-[var(--muted)]">Group Mention Mode</label>
        <select data-field="mentionMode" style=${selectStyle}>
          <option value="mention">Must @mention bot</option>
          <option value="always">Always respond</option>
          <option value="none">Don't respond in groups</option>
        </select>
      `
			}

      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${addModel.value}
        onChange=${(v) => {
					addModel.value = v;
				}}
        placeholder=${defaultPlaceholder} />
      <label class="text-xs text-[var(--muted)]">DM Allowlist (${type === "whatsapp" ? "phone numbers" : "usernames"})</label>
      <${AllowlistInput} value=${allowlistItems.value} onChange=${(v) => {
				allowlistItems.value = v;
			}} />
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn"
        onClick=${onSubmit} disabled=${saving.value}>
        ${saving.value ? "Connecting\u2026" : "Connect Channel"}
      </button>
    </div>
  </${Modal}>`;
}

// ── Edit channel modal ───────────────────────────────────────
function EditChannelModal() {
	var ch = editingChannel.value;
	var error = useSignal("");
	var saving = useSignal(false);
	var editModel = useSignal("");
	var allowlistItems = useSignal([]);
	useEffect(() => {
		editModel.value = ch?.config?.model || "";
		allowlistItems.value = ch?.config?.allowlist || [];
	}, [ch]);
	if (!ch) return null;
	var cfg = ch.config || {};
	var type = ch.type || "telegram";

	function onSave(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		error.value = "";
		saving.value = true;

		var updateConfig = {
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			allowlist: allowlistItems.value,
		};

		if (type === "telegram") {
			updateConfig.token = cfg.token || "";
			updateConfig.mention_mode = form.querySelector("[data-field=mentionMode]").value;
		} else if (type === "whatsapp") {
			updateConfig.phone_number_id = cfg.phone_number_id || "";
			updateConfig.access_token = cfg.access_token || "";
			updateConfig.app_secret = cfg.app_secret || "";
			updateConfig.verify_token = cfg.verify_token || "";
		}

		if (editModel.value) {
			updateConfig.model = editModel.value;
			var found = modelsSig.value.find((x) => x.id === editModel.value);
			if (found?.provider) updateConfig.model_provider = found.provider;
		}
		sendRpc("channels.update", {
			type: type,
			account_id: ch.account_id,
			config: updateConfig,
		}).then((res) => {
			saving.value = false;
			if (res?.ok) {
				editingChannel.value = null;
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to update channel.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	var selectStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;cursor:pointer;";

	var title = type === "whatsapp" ? "Edit WhatsApp Business" : "Edit Telegram Bot";
	var displayName = ch.name || ch.account_id || (type === "whatsapp" ? "WhatsApp" : "Telegram");

	return html`<${Modal} show=${true} onClose=${() => {
		editingChannel.value = null;
	}} title=${title}>
    <div class="channel-form">
      <div class="text-sm text-[var(--text-strong)]">${displayName}</div>
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${selectStyle} value=${cfg.dm_policy || "open"}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>

      ${
				type === "telegram" &&
				html`
        <label class="text-xs text-[var(--muted)]">Group Mention Mode</label>
        <select data-field="mentionMode" style=${selectStyle} value=${cfg.mention_mode || "mention"}>
          <option value="mention">Must @mention bot</option>
          <option value="always">Always respond</option>
          <option value="none">Don't respond in groups</option>
        </select>
      `
			}

      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${editModel.value}
        onChange=${(v) => {
					editModel.value = v;
				}}
        placeholder=${defaultPlaceholder} />
      <label class="text-xs text-[var(--muted)]">DM Allowlist (${type === "whatsapp" ? "phone numbers" : "usernames"})</label>
      <${AllowlistInput} value=${allowlistItems.value} onChange=${(v) => {
				allowlistItems.value = v;
			}} />
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn"
        onClick=${onSave} disabled=${saving.value}>
        ${saving.value ? "Saving\u2026" : "Save Changes"}
      </button>
    </div>
  </${Modal}>`;
}

// ── Main page component ──────────────────────────────────────
function ChannelsPage() {
	useEffect(() => {
		// Use prefetched cache for instant render
		if (S.cachedChannels !== null) channels.value = S.cachedChannels;
		if (connected.value) loadChannels();

		var unsub = onEvent("channel", (p) => {
			if (p.kind === "otp_resolved") {
				loadChannels();
			}
			if (
				activeTab.value === "senders" &&
				sendersAccount.value === p.account_id &&
				(p.kind === "inbound_message" || p.kind === "otp_challenge" || p.kind === "otp_resolved")
			) {
				loadSenders();
			}
		});
		S.setChannelEventUnsub(unsub);

		return () => {
			if (unsub) unsub();
			S.setChannelEventUnsub(null);
		};
	}, [connected.value]);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Channels</h2>
        <div style="display:flex;gap:4px;margin-left:12px;">
          <button class="session-action-btn" style=${activeTab.value === "channels" ? "font-weight:600;" : ""}
            onClick=${() => {
							activeTab.value = "channels";
						}}>Channels</button>
          <button class="session-action-btn" style=${activeTab.value === "senders" ? "font-weight:600;" : ""}
            onClick=${() => {
							activeTab.value = "senders";
						}}>Senders</button>
        </div>
        ${
					activeTab.value === "channels" &&
					html`
          <button class="provider-btn"
            onClick=${() => {
							if (connected.value) showAddModal.value = true;
						}}>+ Add Channel</button>
        `
				}
      </div>
      ${activeTab.value === "channels" ? html`<${ChannelsTab} />` : html`<${SendersTab} />`}
    </div>
    <${AddChannelModal} />
    <${EditChannelModal} />
    <${ConfirmDialog} />
  `;
}

registerPage(
	"/channels",
	function initChannels(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		activeTab.value = "channels";
		showAddModal.value = false;
		editingChannel.value = null;
		sendersAccount.value = "";
		senders.value = [];
		render(html`<${ChannelsPage} />`, container);
	},
	function teardownChannels() {
		S.setRefreshChannelsPage(null);
		if (S.channelEventUnsub) {
			S.channelEventUnsub();
			S.setChannelEventUnsub(null);
		}
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
