//! Outbound message sender for XMPP.
//!
//! Sends stanzas to the event loop via the per-account `stanza_tx` channel.

use {anyhow::Result, async_trait::async_trait, tracing::warn};

use {moltis_channels::plugin::ChannelOutbound, moltis_common::types::ReplyPayload};

use crate::{
    stanza,
    state::AccountStateMap,
    xep::{chat_states, oob},
};

/// Outbound message sender for XMPP.
pub struct XmppOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl XmppOutbound {
    /// Determine the message type based on whether the recipient is a known MUC room.
    async fn msg_type_for(&self, account_id: &str, to: &str) -> &'static str {
        let accounts = self.accounts.read().await;
        if let Some(state) = accounts.get(account_id) {
            // Extract the bare JID (room@server) from the `to` address.
            let bare_to = to.split('/').next().unwrap_or(to);
            if state.config.rooms.iter().any(|r| r == bare_to)
                || state.config.muc_rooms.contains_key(bare_to)
            {
                return "groupchat";
            }
        }
        "chat"
    }

    /// Get the full JID (`jid/resource`) for an account.
    async fn full_jid(&self, account_id: &str) -> Result<String> {
        let accounts = self.accounts.read().await;
        let state = accounts
            .get(account_id)
            .ok_or_else(|| anyhow::anyhow!("unknown xmpp account: {account_id}"))?;
        Ok(format!("{}/{}", state.config.jid, state.config.resource))
    }

    /// Send a raw stanza to the event loop for the given account.
    async fn send_stanza(&self, account_id: &str, stanza: crate::minidom::Element) -> Result<()> {
        let tx = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|s| s.stanza_tx.clone())
                .ok_or_else(|| anyhow::anyhow!("unknown xmpp account: {account_id}"))?
        };

        tx.send(stanza)
            .await
            .map_err(|_| anyhow::anyhow!("xmpp event loop closed for account {account_id}"))
    }
}

#[async_trait]
impl ChannelOutbound for XmppOutbound {
    async fn send_text(&self, account_id: &str, to: &str, text: &str) -> Result<()> {
        let from = self.full_jid(account_id).await?;
        let msg_type = self.msg_type_for(account_id, to).await;

        let accounts = self.accounts.read().await;
        let chunk_limit = accounts
            .get(account_id)
            .map(|s| s.config.text_chunk_limit)
            .unwrap_or(4000);
        drop(accounts);

        let chunks = stanza::chunk_text(text, chunk_limit);

        for chunk in &chunks {
            let el = stanza::build_message(&from, to, msg_type, chunk);
            self.send_stanza(account_id, el).await?;
        }

        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        let from = self.full_jid(account_id).await?;
        let msg_type = self.msg_type_for(account_id, to).await;
        let el =
            chat_states::build_chat_state(&from, to, msg_type, chat_states::ChatState::Composing);
        self.send_stanza(account_id, el).await
    }

    async fn send_media(&self, account_id: &str, to: &str, payload: &ReplyPayload) -> Result<()> {
        let from = self.full_jid(account_id).await?;
        let msg_type = self.msg_type_for(account_id, to).await;

        if let Some(ref media) = payload.media {
            // Send as OOB (out-of-band) URL attachment.
            let description = if payload.text.is_empty() {
                None
            } else {
                Some(payload.text.as_str())
            };
            let el = oob::build_oob_message(&from, to, msg_type, &media.url, description);
            self.send_stanza(account_id, el).await?;
        } else if !payload.text.is_empty() {
            // No media, just text.
            self.send_text(account_id, to, &payload.text).await?;
        } else {
            warn!(account_id, to, "send_media called with empty payload");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{config::XmppAccountConfig, stanza::ns, state::AccountState},
        secrecy::Secret,
        std::sync::{Arc, atomic::AtomicBool},
        tokio::sync::mpsc,
        tokio_util::sync::CancellationToken,
    };

    /// Helper to create an AccountStateMap with one account.
    async fn setup_account(
        rooms: Vec<String>,
    ) -> (AccountStateMap, mpsc::Receiver<crate::minidom::Element>) {
        let (tx, rx) = mpsc::channel(16);
        let map: AccountStateMap =
            Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let state = AccountState {
            account_id: "test".into(),
            config: XmppAccountConfig {
                jid: "bot@example.com".into(),
                password: Secret::new("pass".into()),
                resource: "moltis".into(),
                rooms,
                ..Default::default()
            },
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: None,
            stanza_tx: tx,
            connected: Arc::new(AtomicBool::new(true)),
        };
        map.write().await.insert("test".into(), state);
        (map, rx)
    }

    #[tokio::test]
    async fn send_text_dm() {
        let (accounts, mut rx) = setup_account(vec![]).await;
        let outbound = XmppOutbound { accounts };

        outbound
            .send_text("test", "alice@example.com", "Hello!")
            .await
            .unwrap();

        let el = rx.recv().await.unwrap();
        assert_eq!(el.name(), "message");
        assert_eq!(el.attr("type"), Some("chat"));
        assert_eq!(el.attr("to"), Some("alice@example.com"));
        let body = el.get_child("body", ns::JABBER_CLIENT).unwrap();
        assert_eq!(body.text(), "Hello!");
    }

    #[tokio::test]
    async fn send_text_groupchat() {
        let (accounts, mut rx) = setup_account(vec!["room@conference.example.com".into()]).await;
        let outbound = XmppOutbound { accounts };

        outbound
            .send_text("test", "room@conference.example.com", "Hi room!")
            .await
            .unwrap();

        let el = rx.recv().await.unwrap();
        assert_eq!(el.attr("type"), Some("groupchat"));
    }

    #[tokio::test]
    async fn send_typing_composing() {
        let (accounts, mut rx) = setup_account(vec![]).await;
        let outbound = XmppOutbound { accounts };

        outbound
            .send_typing("test", "alice@example.com")
            .await
            .unwrap();

        let el = rx.recv().await.unwrap();
        assert_eq!(el.name(), "message");
        let composing = el.get_child("composing", ns::CHAT_STATES);
        assert!(composing.is_some());
    }

    #[tokio::test]
    async fn send_media_oob() {
        let (accounts, mut rx) = setup_account(vec![]).await;
        let outbound = XmppOutbound { accounts };

        let payload = ReplyPayload {
            text: "Check this out".into(),
            media: Some(moltis_common::types::MediaAttachment {
                url: "https://example.com/image.png".into(),
                mime_type: "image/png".into(),
            }),
            reply_to_id: None,
            silent: false,
        };

        outbound
            .send_media("test", "alice@example.com", &payload)
            .await
            .unwrap();

        let el = rx.recv().await.unwrap();
        assert_eq!(el.name(), "message");
        // Should have OOB extension.
        let x = el.get_child("x", ns::OOB);
        assert!(x.is_some());
    }

    #[tokio::test]
    async fn send_text_unknown_account() {
        let (accounts, _rx) = setup_account(vec![]).await;
        let outbound = XmppOutbound { accounts };

        let result = outbound
            .send_text("nonexistent", "to@example.com", "hi")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown"));
    }
}
