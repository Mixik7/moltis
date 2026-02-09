//! Outbound message sending for WhatsApp Web.

use std::sync::Arc;

use {
    anyhow::Result,
    async_trait::async_trait,
    tokio::sync::RwLock,
    tracing::{debug, warn},
    uuid::Uuid,
};

use {moltis_channels::plugin::ChannelOutbound, moltis_common::types::ReplyPayload};

use crate::{sidecar::SidecarHandle, types::GatewayMessage};

/// WhatsApp Web outbound message sender.
pub struct WhatsAppOutbound {
    sidecar: Arc<RwLock<Option<SidecarHandle>>>,
}

impl WhatsAppOutbound {
    /// Create a new outbound sender.
    pub fn new(sidecar: Arc<RwLock<Option<SidecarHandle>>>) -> Self {
        Self { sidecar }
    }

    async fn send_to_sidecar(&self, account_id: &str, msg: GatewayMessage) -> Result<()> {
        let handle = self.sidecar.read().await;
        match handle.as_ref() {
            Some(h) => h.send(msg).await,
            None => {
                warn!(account_id, "sidecar not connected, cannot send message");
                Err(anyhow::anyhow!("sidecar not connected"))
            },
        }
    }
}

#[async_trait]
impl ChannelOutbound for WhatsAppOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        let request_id = Uuid::new_v4().to_string();
        debug!(account_id, to, request_id, "sending text message");

        self.send_to_sidecar(account_id, GatewayMessage::SendText {
            account_id: account_id.to_string(),
            to: to.to_string(),
            text: text.to_string(),
            request_id,
        })
        .await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        debug!(account_id, to, "sending typing indicator");

        self.send_to_sidecar(account_id, GatewayMessage::SendTyping {
            account_id: account_id.to_string(),
            to: to.to_string(),
        })
        .await
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        if let Some(ref media) = payload.media {
            let request_id = Uuid::new_v4().to_string();
            let media_type = if media.mime_type.starts_with("image/") {
                "image"
            } else if media.mime_type.starts_with("video/") {
                "video"
            } else if media.mime_type.starts_with("audio/") {
                "audio"
            } else {
                "document"
            };

            debug!(
                account_id,
                to, request_id, media_type, "sending media message"
            );

            self.send_to_sidecar(account_id, GatewayMessage::SendMedia {
                account_id: account_id.to_string(),
                to: to.to_string(),
                media_url: media.url.clone(),
                media_type: media_type.to_string(),
                caption: if payload.text.is_empty() {
                    None
                } else {
                    Some(payload.text.clone())
                },
                request_id,
            })
            .await
        } else if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, None).await
        } else {
            Ok(())
        }
    }
}
