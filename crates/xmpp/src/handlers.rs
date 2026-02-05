//! Inbound XMPP stanza routing.
//!
//! Dispatches incoming messages, presence, and IQ stanzas from the event loop
//! to the appropriate handlers (access control, message logging, chat dispatch).

use std::sync::Arc;

use {
    tokio_xmpp::Stanza,
    tracing::{debug, warn},
};

use {
    moltis_channels::{
        ChannelEvent, ChannelEventSink, ChannelMessageMeta, ChannelOutbound, ChannelReplyTarget,
        message_log::{MessageLog, MessageLogEntry},
    },
    moltis_common::types::ChatType,
};

use crate::{access, config::XmppAccountConfig, outbound::XmppOutbound, state::AccountStateMap};

/// Route an inbound stanza to the appropriate handler.
pub async fn handle_stanza(
    account_id: &str,
    config: &XmppAccountConfig,
    stanza: Stanza,
    accounts: &AccountStateMap,
    message_log: Option<&Arc<dyn MessageLog>>,
    event_sink: Option<&Arc<dyn ChannelEventSink>>,
) {
    match stanza {
        Stanza::Message(msg) => {
            handle_message(account_id, config, msg, accounts, message_log, event_sink).await;
        },
        Stanza::Presence(pres) => {
            handle_presence(account_id, config, pres).await;
        },
        Stanza::Iq(iq) => {
            handle_iq(account_id, iq).await;
        },
    }
}

/// Handle an inbound `<message>` stanza.
async fn handle_message(
    account_id: &str,
    config: &XmppAccountConfig,
    msg: tokio_xmpp::parsers::message::Message,
    accounts: &AccountStateMap,
    message_log: Option<&Arc<dyn MessageLog>>,
    event_sink: Option<&Arc<dyn ChannelEventSink>>,
) {
    // Extract body text.
    let body = match msg
        .bodies
        .get(&tokio_xmpp::parsers::message::Lang::default())
    {
        Some(body) => body.clone(),
        None => {
            // Try the first body if default lang not found.
            match msg.bodies.values().next() {
                Some(body) => body.clone(),
                None => {
                    debug!(account_id, "ignoring message without body");
                    return;
                },
            }
        },
    };

    // Extract sender JID.
    let from = match msg.from {
        Some(ref jid) => jid.to_string(),
        None => {
            debug!(account_id, "ignoring message without from");
            return;
        },
    };

    // Determine chat type from message type.
    let (chat_type, chat_id, peer_jid, room_jid) = match msg.type_ {
        tokio_xmpp::parsers::message::MessageType::Groupchat => {
            // MUC message: from is room@server/nick
            let from_str = from.as_str();

            // Skip self-echo: if the nick matches our resource, ignore.
            if let Some(nick) = from_str.split('/').nth(1)
                && nick == config.resource
            {
                debug!(account_id, "skipping own MUC echo");
                return;
            }

            let room = from_str.split('/').next().unwrap_or(&from);
            let peer = extract_bare_jid(&from);
            (
                ChatType::Group,
                room.to_string(),
                peer,
                Some(room.to_string()),
            )
        },
        _ => {
            // 1:1 chat or other message type.
            let peer = extract_bare_jid(&from);
            let chat_id = peer.clone();
            (ChatType::Dm, chat_id, peer, None)
        },
    };

    // Check if the bot was mentioned in the message.
    let bot_mentioned = check_bot_mentioned(&body, &config.jid, &config.resource);

    debug!(
        account_id,
        ?chat_type,
        %peer_jid,
        ?bot_mentioned,
        "checking access"
    );

    // Access control.
    let access_result = access::check_access(
        config,
        &chat_type,
        &peer_jid,
        room_jid.as_deref(),
        bot_mentioned,
    );
    let access_granted = access_result.is_ok();

    // Extract sender display name (nick from MUC or local part of JID).
    let sender_name = match chat_type {
        ChatType::Group => from.split('/').nth(1).map(String::from),
        _ => Some(extract_local_part(&peer_jid)),
    };

    let username = Some(peer_jid.clone());

    // Log every inbound message (before returning on denial).
    if let Some(log) = message_log {
        let chat_type_str = match chat_type {
            ChatType::Dm => "dm",
            ChatType::Group => "group",
            ChatType::Channel => "channel",
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let entry = MessageLogEntry {
            id: 0,
            account_id: account_id.to_string(),
            channel_type: "xmpp".into(),
            peer_id: peer_jid.clone(),
            username: username.clone(),
            sender_name: sender_name.clone(),
            chat_id: chat_id.clone(),
            chat_type: chat_type_str.into(),
            body: body.clone(),
            access_granted,
            created_at: now,
        };
        if let Err(e) = log.log(entry).await {
            warn!(account_id, "failed to log message: {e}");
        }
    }

    // Emit channel event for real-time UI updates.
    if let Some(sink) = event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: "xmpp".into(),
            account_id: account_id.to_string(),
            peer_id: peer_jid.clone(),
            username: username.clone(),
            sender_name: sender_name.clone(),
            message_count: None,
            access_granted,
        })
        .await;
    }

    if let Err(reason) = access_result {
        warn!(account_id, %reason, %peer_jid, "handler: access denied");
        return;
    }

    debug!(account_id, "handler: access granted");

    // Strip bot mention from the body for cleaner dispatch.
    let cleaned_body = strip_mention(&body, &config.jid, &config.resource);

    // Dispatch to the chat session.
    if let Some(sink) = event_sink
        && !cleaned_body.is_empty()
    {
        let reply_target = ChannelReplyTarget {
            channel_type: "xmpp".into(),
            account_id: account_id.to_string(),
            chat_id: chat_id.clone(),
        };

        // Intercept slash commands before dispatching to the LLM.
        if cleaned_body.starts_with('/') {
            let cmd_text = cleaned_body.trim_start_matches('/');
            let cmd = cmd_text.split_whitespace().next().unwrap_or("");
            if matches!(
                cmd,
                "new" | "clear" | "compact" | "context" | "model" | "sandbox" | "sessions" | "help"
            ) {
                let response = if cmd == "help" {
                    "Available commands:\n/new — Start a new session\n/sessions — List and switch sessions\n/model — Switch provider/model\n/sandbox — Toggle sandbox and choose image\n/clear — Clear session history\n/compact — Compact session (summarize)\n/context — Show session context info\n/help — Show this help"
                        .to_string()
                } else {
                    match sink.dispatch_command(cmd_text, reply_target.clone()).await {
                        Ok(msg) => msg,
                        Err(e) => format!("Error: {e}"),
                    }
                };

                // Send response back via outbound.
                let outbound = XmppOutbound {
                    accounts: accounts.clone(),
                };
                if let Err(e) = outbound
                    .send_text(account_id, &reply_target.chat_id, &response)
                    .await
                {
                    warn!(account_id, "failed to send command response: {e}");
                }
                return;
            }
        }

        let meta = ChannelMessageMeta {
            channel_type: "xmpp".into(),
            sender_name: sender_name.clone(),
            username: username.clone(),
            model: config.model.clone(),
        };
        sink.dispatch_to_chat(&cleaned_body, reply_target, meta)
            .await;
    }
}

/// Handle an inbound `<presence>` stanza.
async fn handle_presence(
    account_id: &str,
    _config: &XmppAccountConfig,
    pres: tokio_xmpp::parsers::presence::Presence,
) {
    let from = pres
        .from
        .as_ref()
        .map(|j| j.to_string())
        .unwrap_or_default();

    match pres.type_ {
        tokio_xmpp::parsers::presence::Type::None => {
            debug!(account_id, %from, "presence: available");
        },
        tokio_xmpp::parsers::presence::Type::Unavailable => {
            debug!(account_id, %from, "presence: unavailable");
        },
        tokio_xmpp::parsers::presence::Type::Error => {
            warn!(account_id, %from, "presence: error");
        },
        other => {
            debug!(account_id, %from, ?other, "presence: other type");
        },
    }
}

/// Handle an inbound `<iq>` stanza.
async fn handle_iq(account_id: &str, iq: tokio_xmpp::parsers::iq::Iq) {
    debug!(
        account_id,
        id = iq.id(),
        from = ?iq.from(),
        "iq stanza (unhandled)"
    );
}

/// Extract the bare JID (user@server) from a full JID (user@server/resource).
fn extract_bare_jid(full_jid: &str) -> String {
    full_jid.split('/').next().unwrap_or(full_jid).to_string()
}

/// Extract the local part (user) from a bare JID (user@server).
fn extract_local_part(bare_jid: &str) -> String {
    bare_jid.split('@').next().unwrap_or(bare_jid).to_string()
}

/// Check if the bot was @mentioned in the message body.
///
/// Checks for mentions of the bare JID, local part, or resource name.
fn check_bot_mentioned(body: &str, bot_jid: &str, resource: &str) -> bool {
    let body_lower = body.to_lowercase();
    let jid_lower = bot_jid.to_lowercase();
    let local = extract_local_part(bot_jid).to_lowercase();
    let resource_lower = resource.to_lowercase();

    // Check for @jid, @local, or @resource mention.
    body_lower.contains(&jid_lower)
        || body_lower.contains(&format!("@{local}"))
        || body_lower.contains(&format!("@{resource_lower}"))
        || body_lower.contains(&local) // Many XMPP clients just use the nick
}

/// Strip bot mention from the message body.
fn strip_mention(body: &str, bot_jid: &str, resource: &str) -> String {
    let local = extract_local_part(bot_jid);

    // Try stripping common mention patterns.
    let patterns = [
        format!("@{bot_jid}"),
        format!("@{local}"),
        format!("@{resource}"),
        bot_jid.to_string(),
        local.clone(),
        resource.to_string(),
    ];

    let mut result = body.to_string();
    for pattern in &patterns {
        // Case-insensitive removal — only strip the first occurrence.
        if let Some(pos) = result.to_lowercase().find(&pattern.to_lowercase()) {
            result = format!("{}{}", &result[..pos], &result[pos + pattern.len()..]);
            break;
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_bare_jid_from_full() {
        assert_eq!(
            extract_bare_jid("user@example.com/resource"),
            "user@example.com"
        );
    }

    #[test]
    fn extract_bare_jid_already_bare() {
        assert_eq!(extract_bare_jid("user@example.com"), "user@example.com");
    }

    #[test]
    fn extract_local_part_works() {
        assert_eq!(extract_local_part("user@example.com"), "user");
    }

    #[test]
    fn bot_mentioned_by_local_part() {
        assert!(check_bot_mentioned(
            "hey bot, how are you?",
            "bot@example.com",
            "moltis"
        ));
    }

    #[test]
    fn bot_mentioned_by_at_local() {
        assert!(check_bot_mentioned(
            "hello @bot, what's up?",
            "bot@example.com",
            "moltis"
        ));
    }

    #[test]
    fn bot_mentioned_by_resource() {
        assert!(check_bot_mentioned(
            "hey @moltis can you help?",
            "bot@example.com",
            "moltis"
        ));
    }

    #[test]
    fn bot_not_mentioned() {
        assert!(!check_bot_mentioned(
            "hello everyone",
            "some-unique-bot-name@example.com",
            "some-unique-resource-name"
        ));
    }

    #[test]
    fn strip_at_mention() {
        assert_eq!(
            strip_mention("@bot hello there", "bot@example.com", "moltis"),
            "hello there"
        );
    }

    #[test]
    fn strip_jid_mention() {
        assert_eq!(
            strip_mention("bot@example.com can you help?", "bot@example.com", "moltis"),
            "can you help?"
        );
    }

    #[test]
    fn strip_no_mention() {
        assert_eq!(
            strip_mention("just a normal message", "bot@example.com", "moltis"),
            "just a normal message"
        );
    }

    #[test]
    fn classify_dm_chat_id() {
        let peer = extract_bare_jid("alice@example.com/phone");
        assert_eq!(peer, "alice@example.com");
    }

    #[test]
    fn classify_muc_room() {
        let from = "room@conference.example.com/alice";
        let room = from.split('/').next().unwrap();
        let nick = from.split('/').nth(1).unwrap();
        assert_eq!(room, "room@conference.example.com");
        assert_eq!(nick, "alice");
    }
}
