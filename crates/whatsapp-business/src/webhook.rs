//! WhatsApp webhook handling.

use std::sync::Arc;

use {
    hmac::{Hmac, Mac},
    sha2::Sha256,
    tracing::{debug, warn},
};

use {
    moltis_channels::{
        ChannelEvent, ChannelMessageMeta, ChannelOutbound, ChannelReplyTarget, ChannelType,
        message_log::MessageLogEntry,
    },
    moltis_common::types::ChatType,
};

use crate::{access, config::WhatsAppAccountConfig, state::AccountStateMap, types::WebhookPayload};

type HmacSha256 = Hmac<Sha256>;

/// Verify the webhook signature from WhatsApp.
///
/// The signature is sent in the `X-Hub-Signature-256` header as `sha256=<hex>`.
pub fn verify_signature(body: &[u8], signature_header: &str, app_secret: &str) -> bool {
    let expected = match signature_header.strip_prefix("sha256=") {
        Some(hex) => hex,
        None => {
            warn!("invalid signature header format (missing sha256= prefix)");
            return false;
        },
    };

    let mut mac = match HmacSha256::new_from_slice(app_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            warn!("failed to create HMAC");
            return false;
        },
    };

    mac.update(body);
    let computed = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison to prevent timing attacks.
    constant_time_eq(&computed, expected)
}

/// Constant-time string comparison.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Process a webhook payload and dispatch messages to the chat system.
pub async fn process_webhook(
    account_id: &str,
    accounts: &AccountStateMap,
    payload: WebhookPayload,
) {
    let (config, message_log, event_sink) = {
        let accts = accounts.read().unwrap();
        match accts.get(account_id) {
            Some(state) => (
                state.config.clone(),
                state.message_log.clone(),
                state.event_sink.clone(),
            ),
            None => {
                warn!(account_id, "webhook: account not found in state map");
                return;
            },
        }
    };

    for entry in payload.entry {
        for change in entry.changes {
            if change.field != "messages" {
                debug!(account_id, field = %change.field, "ignoring non-message webhook");
                continue;
            }

            let value = change.value;

            // Get phone_number_id from metadata to verify it matches our config.
            if let Some(ref metadata) = value.metadata
                && metadata.phone_number_id != config.phone_number_id
            {
                warn!(
                    account_id,
                    expected = %config.phone_number_id,
                    received = %metadata.phone_number_id,
                    "phone number ID mismatch"
                );
                continue;
            }

            // Build a contact lookup map.
            let contacts: std::collections::HashMap<String, String> = value
                .contacts
                .iter()
                .filter_map(|c| {
                    c.profile
                        .as_ref()
                        .map(|p| (c.wa_id.clone(), p.name.clone()))
                })
                .collect();

            for msg in value.messages {
                let text = match msg.text_body() {
                    Some(t) if !t.is_empty() => t,
                    _ => {
                        if !msg.has_media() {
                            debug!(account_id, msg_type = %msg.message_type, "ignoring non-text, non-media message");
                            continue;
                        }
                        String::new()
                    },
                };

                let peer_id = msg.from.clone();
                let sender_name = contacts.get(&peer_id).cloned();

                // For now, treat all WhatsApp messages as DMs.
                // Group support would require checking if the message is from a group.
                let chat_type = ChatType::Dm;
                let group_id: Option<&str> = None;

                debug!(
                    account_id,
                    peer_id,
                    ?sender_name,
                    "processing whatsapp message"
                );

                // Access control.
                let access_result = access::check_access(&config, &chat_type, &peer_id, group_id);
                let access_granted = access_result.is_ok();

                // Log every inbound message.
                if let Some(ref log) = message_log {
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
                        channel_type: ChannelType::Whatsapp.to_string(),
                        peer_id: peer_id.clone(),
                        username: None, // WhatsApp doesn't have usernames
                        sender_name: sender_name.clone(),
                        chat_id: peer_id.clone(), // Use peer_id as chat_id for DMs
                        chat_type: chat_type_str.into(),
                        body: text.clone(),
                        access_granted,
                        created_at: now,
                    };
                    if let Err(e) = log.log(entry).await {
                        warn!(account_id, "failed to log message: {e}");
                    }
                }

                // Emit channel event for real-time UI updates.
                if let Some(ref sink) = event_sink {
                    sink.emit(ChannelEvent::InboundMessage {
                        channel_type: ChannelType::Whatsapp,
                        account_id: account_id.to_string(),
                        peer_id: peer_id.clone(),
                        username: None,
                        sender_name: sender_name.clone(),
                        message_count: None,
                        access_granted,
                    })
                    .await;
                }

                if let Err(reason) = access_result {
                    warn!(account_id, %reason, peer_id, "webhook: access denied");
                    continue;
                }

                debug!(account_id, "webhook: access granted");

                // Dispatch to the chat session.
                if let Some(ref sink) = event_sink
                    && !text.is_empty()
                {
                    let reply_target = ChannelReplyTarget {
                        channel_type: ChannelType::Whatsapp,
                        account_id: account_id.to_string(),
                        chat_id: peer_id.clone(),
                        message_id: None,
                    };

                    // Intercept slash commands.
                    if text.starts_with('/') {
                        let cmd_text = text.trim_start_matches('/');
                        let cmd = cmd_text.split_whitespace().next().unwrap_or("");
                        if matches!(
                            cmd,
                            "new" | "clear" | "compact" | "context" | "model" | "help"
                        ) {
                            let response = if cmd == "help" {
                                "Available commands:\n/new — Start a new session\n/model — Switch provider/model\n/clear — Clear session history\n/compact — Compact session (summarize)\n/context — Show session context info\n/help — Show this help".to_string()
                            } else {
                                match sink.dispatch_command(cmd_text, reply_target.clone()).await {
                                    Ok(m) => m,
                                    Err(e) => format!("Error: {e}"),
                                }
                            };

                            // Send the response back via outbound.
                            let outbound = {
                                let accts = accounts.read().unwrap();
                                accts.get(account_id).map(|s| Arc::clone(&s.outbound))
                            };
                            if let Some(outbound) = outbound
                                && let Err(e) = outbound
                                    .send_text(account_id, &reply_target.chat_id, &response, None)
                                    .await
                            {
                                warn!(account_id, "failed to send command response: {e}");
                            }
                            continue;
                        }
                    }

                    let meta = ChannelMessageMeta {
                        channel_type: ChannelType::Whatsapp,
                        sender_name,
                        username: None,
                        message_kind: None,
                        model: config.model.clone(),
                    };
                    sink.dispatch_to_chat(&text, reply_target, meta).await;
                }
            }
        }
    }
}

/// Verify webhook subscription (GET request).
///
/// WhatsApp sends a GET request with:
/// - `hub.mode=subscribe`
/// - `hub.verify_token=<your_verify_token>`
/// - `hub.challenge=<random_string>`
///
/// Returns `Some(challenge)` if verification succeeds.
pub fn verify_webhook_subscription(
    mode: Option<&str>,
    token: Option<&str>,
    challenge: Option<&str>,
    config: &WhatsAppAccountConfig,
) -> Option<String> {
    let mode = mode?;
    let token = token?;
    let challenge = challenge?;

    if mode == "subscribe" && token == config.verify_token {
        Some(challenge.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_signature_valid() {
        let body = b"test body";
        let secret = "test_secret";

        // Compute expected signature.
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        assert!(verify_signature(body, &expected, secret));
    }

    #[test]
    fn test_verify_signature_invalid() {
        let body = b"test body";
        let secret = "test_secret";
        let wrong_signature =
            "sha256=0000000000000000000000000000000000000000000000000000000000000000";

        assert!(!verify_signature(body, wrong_signature, secret));
    }

    #[test]
    fn test_verify_signature_missing_prefix() {
        let body = b"test body";
        let secret = "test_secret";

        assert!(!verify_signature(body, "invalid_format", secret));
    }

    #[test]
    fn test_verify_webhook_subscription_valid() {
        let config = WhatsAppAccountConfig {
            verify_token: "my_token".into(),
            ..Default::default()
        };

        let result = verify_webhook_subscription(
            Some("subscribe"),
            Some("my_token"),
            Some("challenge_123"),
            &config,
        );

        assert_eq!(result, Some("challenge_123".to_string()));
    }

    #[test]
    fn test_verify_webhook_subscription_invalid_token() {
        let config = WhatsAppAccountConfig {
            verify_token: "my_token".into(),
            ..Default::default()
        };

        let result = verify_webhook_subscription(
            Some("subscribe"),
            Some("wrong_token"),
            Some("challenge_123"),
            &config,
        );

        assert_eq!(result, None);
    }

    #[test]
    fn test_verify_webhook_subscription_wrong_mode() {
        let config = WhatsAppAccountConfig {
            verify_token: "my_token".into(),
            ..Default::default()
        };

        let result = verify_webhook_subscription(
            Some("unsubscribe"),
            Some("my_token"),
            Some("challenge_123"),
            &config,
        );

        assert_eq!(result, None);
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("", "a"));
    }
}
