// ── SessionHeader Preact component ───────────────────────────
//
// Replaces the imperative updateChatSessionHeader() with a reactive
// Preact component reading sessionStore.activeSession.

import { html } from "htm/preact";
import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import * as gon from "../gon.js";
import { sendRpc } from "../helpers.js";
import { clearActiveSession, fetchSessions, switchSession } from "../sessions.js";
import { sessionStore } from "../stores/session-store.js";
import { confirmDialog } from "../ui.js";

function nextSessionKey(currentKey) {
	var allSessions = sessionStore.sessions.value;
	var s = allSessions.find((x) => x.key === currentKey);
	if (s?.parentSessionKey) return s.parentSessionKey;
	var idx = allSessions.findIndex((x) => x.key === currentKey);
	if (idx >= 0 && idx + 1 < allSessions.length) return allSessions[idx + 1].key;
	if (idx > 0) return allSessions[idx - 1].key;
	return "main";
}

export function SessionHeader() {
	var session = sessionStore.activeSession.value;
	var currentKey = sessionStore.activeSessionKey.value;

	var [renaming, setRenaming] = useState(false);
	var [clearing, setClearing] = useState(false);
	var inputRef = useRef(null);

	var fullName = session ? session.label || session.key : currentKey;
	var displayName = fullName.length > 20 ? `${fullName.slice(0, 20)}\u2026` : fullName;

	var isMain = currentKey === "main";
	var isChannel = session?.channelBinding || currentKey.startsWith("telegram:");
	var isCron = currentKey.startsWith("cron:");
	var canRename = !(isMain || isChannel || isCron);

	var startRename = useCallback(() => {
		if (!canRename) return;
		setRenaming(true);
		requestAnimationFrame(() => {
			if (inputRef.current) {
				inputRef.current.value = fullName;
				inputRef.current.focus();
				inputRef.current.select();
			}
		});
	}, [canRename, fullName]);

	var commitRename = useCallback(() => {
		var val = inputRef.current?.value.trim() || "";
		setRenaming(false);
		if (val && val !== fullName) {
			sendRpc("sessions.patch", { key: currentKey, label: val }).then((res) => {
				if (res?.ok) fetchSessions();
			});
		}
	}, [currentKey, fullName]);

	var onKeyDown = useCallback(
		(e) => {
			if (e.key === "Enter") {
				e.preventDefault();
				commitRename();
			}
			if (e.key === "Escape") {
				setRenaming(false);
			}
		},
		[commitRename],
	);

	var onFork = useCallback(() => {
		sendRpc("sessions.fork", { key: currentKey }).then((res) => {
			if (res?.ok && res.payload?.sessionKey) {
				fetchSessions();
				switchSession(res.payload.sessionKey);
			}
		});
	}, [currentKey]);

	var onDelete = useCallback(() => {
		var msgCount = session ? session.messageCount || 0 : 0;
		var nextKey = nextSessionKey(currentKey);
		var doDelete = () => {
			sendRpc("sessions.delete", { key: currentKey }).then((res) => {
				if (res && !res.ok && res.error && res.error.indexOf("uncommitted changes") !== -1) {
					confirmDialog("Worktree has uncommitted changes. Force delete?").then((yes) => {
						if (!yes) return;
						sendRpc("sessions.delete", { key: currentKey, force: true }).then(() => {
							switchSession(nextKey);
							fetchSessions();
						});
					});
					return;
				}
				switchSession(nextKey);
				fetchSessions();
			});
		};
		var isUnmodifiedFork = session && session.forkPoint != null && msgCount <= session.forkPoint;
		if (msgCount > 0 && !isUnmodifiedFork) {
			confirmDialog("Delete this session?").then((yes) => {
				if (yes) doDelete();
			});
		} else {
			doDelete();
		}
	}, [currentKey, session]);

	var onClear = useCallback(() => {
		if (clearing) return;
		setClearing(true);
		clearActiveSession().finally(() => {
			setClearing(false);
		});
	}, [clearing]);

	// ── Agent selector ──────────────────────────────────────
	var agents = gon.get("agents") || [];
	var currentAgentId = session?.agentId || "main";
	var [agentId, setAgentId] = useState(currentAgentId);

	useEffect(() => {
		setAgentId(session?.agentId || "main");
	}, [session?.agentId]);

	var onAgentChange = useCallback(
		(e) => {
			var newId = e.target.value;
			setAgentId(newId);
			sendRpc("agents.set_session", {
				session_key: currentKey,
				agent_id: newId === "main" ? null : newId,
			}).then((res) => {
				if (res?.ok) fetchSessions();
			});
		},
		[currentKey],
	);

	var _currentAgent = agents.find((a) => a.id === agentId);
	var showAgentSelector = agents.length > 1;

	return html`
		<div class="flex items-center gap-2">
			${
				showAgentSelector &&
				html`<select
					class="chat-session-btn"
					style="font-size:0.7rem;padding:2px 6px;cursor:pointer;"
					value=${agentId}
					onChange=${onAgentChange}
					title="Switch agent persona"
				>
					${agents.map(
						(a) => html`<option key=${a.id} value=${a.id}>
							${a.emoji ? `${a.emoji} ` : ""}${a.name}
						</option>`,
					)}
				</select>`
			}
			${
				renaming
					? html`<input
						ref=${inputRef}
						class="chat-session-rename-input"
						onBlur=${commitRename}
						onKeyDown=${onKeyDown}
					/>`
					: html`<span
						class="chat-session-name"
						style=${{ cursor: canRename ? "pointer" : "default" }}
						title=${canRename ? "Click to rename" : ""}
						onClick=${startRename}
					>${displayName}</span>`
			}
			${
				!isCron &&
				html`
				<button class="chat-session-btn" onClick=${onFork} title="Fork session">
					Fork
				</button>
			`
			}
			${
				isMain &&
				html`
				<button class="chat-session-btn" onClick=${onClear} title="Clear session" disabled=${clearing}>
					${clearing ? "Clearing\u2026" : "Clear"}
				</button>
			`
			}
			${
				!(isMain || isCron) &&
				html`
				<button class="chat-session-btn chat-session-btn-danger" onClick=${onDelete} title="Delete session">
					Delete
				</button>
			`
			}
		</div>
	`;
}
