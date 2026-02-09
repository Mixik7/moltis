//! Outbound message sending for WhatsApp.

use {
    anyhow::Result,
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tracing::{debug, warn},
};

use {moltis_channels::plugin::ChannelOutbound, moltis_common::types::ReplyPayload};

use crate::{
    state::AccountStateMap,
    types::{DocumentObject, MediaObject, OutboundMediaContent, SendMediaRequest, SendTextRequest},
};

/// WhatsApp message length limit (approximately 4096 characters).
const WHATSAPP_MAX_MESSAGE_LEN: usize = 4096;

/// Outbound message sender for WhatsApp.
pub struct WhatsAppOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl WhatsAppOutbound {
    /// Create a new outbound sender.
    pub fn new(accounts: AccountStateMap) -> Self {
        Self { accounts }
    }

    /// Send a text message to WhatsApp, chunking if necessary.
    async fn send_text_internal(&self, account_id: &str, to: &str, text: &str) -> Result<()> {
        let (config, client) = {
            let accounts = self.accounts.read().unwrap();
            let state = accounts
                .get(account_id)
                .ok_or_else(|| anyhow::anyhow!("unknown account: {account_id}"))?;
            (state.config.clone(), state.http_client.clone())
        };

        let chunks = chunk_message(text, WHATSAPP_MAX_MESSAGE_LEN);
        let url = config.messages_url();

        for chunk in chunks {
            let request = SendTextRequest::new(to.to_string(), chunk);
            let response = client
                .post(&url)
                .header(
                    "Authorization",
                    format!("Bearer {}", config.access_token.expose_secret()),
                )
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                warn!(account_id, to, %status, "WhatsApp API error: {body}");
                return Err(anyhow::anyhow!("WhatsApp API error: {status}"));
            }

            debug!(account_id, to, "message sent successfully");
        }

        Ok(())
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
        self.send_text_internal(account_id, to, text).await
    }

    async fn send_typing(&self, _account_id: &str, _to: &str) -> Result<()> {
        // WhatsApp doesn't have a typing indicator API.
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        let (config, client) = {
            let accounts = self.accounts.read().unwrap();
            let state = accounts
                .get(account_id)
                .ok_or_else(|| anyhow::anyhow!("unknown account: {account_id}"))?;
            (state.config.clone(), state.http_client.clone())
        };

        if let Some(ref media) = payload.media {
            let url = config.messages_url();

            let (message_type, media_content) = match media.mime_type.as_str() {
                t if t.starts_with("image/") => ("image", OutboundMediaContent::Image {
                    image: MediaObject {
                        link: Some(media.url.clone()),
                        id: None,
                        caption: if payload.text.is_empty() {
                            None
                        } else {
                            Some(payload.text.clone())
                        },
                    },
                }),
                t if t.starts_with("audio/") => ("audio", OutboundMediaContent::Audio {
                    audio: MediaObject {
                        link: Some(media.url.clone()),
                        id: None,
                        caption: None, // Audio doesn't support captions in WhatsApp
                    },
                }),
                t if t.starts_with("video/") => ("video", OutboundMediaContent::Video {
                    video: MediaObject {
                        link: Some(media.url.clone()),
                        id: None,
                        caption: if payload.text.is_empty() {
                            None
                        } else {
                            Some(payload.text.clone())
                        },
                    },
                }),
                _ => ("document", OutboundMediaContent::Document {
                    document: DocumentObject {
                        link: Some(media.url.clone()),
                        id: None,
                        caption: if payload.text.is_empty() {
                            None
                        } else {
                            Some(payload.text.clone())
                        },
                        filename: None, // Filename not available from MediaAttachment
                    },
                }),
            };

            let request = SendMediaRequest {
                messaging_product: "whatsapp",
                recipient_type: "individual",
                to: to.to_string(),
                message_type: message_type.to_string(),
                media: media_content,
            };

            let response = client
                .post(&url)
                .header(
                    "Authorization",
                    format!("Bearer {}", config.access_token.expose_secret()),
                )
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                warn!(account_id, to, %status, "WhatsApp API error: {body}");
                return Err(anyhow::anyhow!("WhatsApp API error: {status}"));
            }

            debug!(account_id, to, "media message sent successfully");
        } else if !payload.text.is_empty() {
            self.send_text_internal(account_id, to, &payload.text)
                .await?;
        }

        Ok(())
    }
}

/// Split a message into chunks that fit within the WhatsApp message limit.
fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find a good break point (prefer newlines, then spaces).
        let chunk_end = find_break_point(remaining, max_len);
        chunks.push(remaining[..chunk_end].to_string());
        remaining = remaining[chunk_end..].trim_start();
    }

    chunks
}

/// Find a good break point in the text.
fn find_break_point(text: &str, max_len: usize) -> usize {
    let search_range = &text[..max_len];

    // Try to break at a newline.
    if let Some(pos) = search_range.rfind('\n') {
        return pos + 1;
    }

    // Try to break at a space.
    if let Some(pos) = search_range.rfind(' ') {
        return pos + 1;
    }

    // No good break point; just cut at max_len.
    max_len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_message_short() {
        let chunks = chunk_message("Hello, world!", 100);
        assert_eq!(chunks, vec!["Hello, world!"]);
    }

    #[test]
    fn test_chunk_message_long() {
        let text = "a".repeat(10);
        let chunks = chunk_message(&text, 4);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "aaaa");
        assert_eq!(chunks[1], "aaaa");
        assert_eq!(chunks[2], "aa");
    }

    #[test]
    fn test_chunk_message_with_spaces() {
        let text = "hello world this is a test";
        let chunks = chunk_message(text, 12);
        assert_eq!(chunks[0], "hello world ");
        assert_eq!(chunks[1], "this is a ");
        assert_eq!(chunks[2], "test");
    }

    #[test]
    fn test_chunk_message_with_newlines() {
        let text = "line1\nline2\nline3";
        let chunks = chunk_message(text, 8);
        assert_eq!(chunks[0], "line1\n");
        assert_eq!(chunks[1], "line2\n");
        assert_eq!(chunks[2], "line3");
    }
}
