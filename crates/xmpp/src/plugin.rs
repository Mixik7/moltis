//! XMPP channel plugin implementation.
//!
//! Mirrors the structure of the Telegram plugin: manages multiple XMPP
//! accounts, each running an event loop in its own task.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use {
    anyhow::Result,
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tracing::{info, warn},
};

use moltis_channels::{
    ChannelEventSink,
    message_log::MessageLog,
    plugin::{ChannelHealthSnapshot, ChannelOutbound, ChannelPlugin, ChannelStatus},
};

use crate::{client, config::XmppAccountConfig, outbound::XmppOutbound, state::AccountStateMap};

/// Cache TTL for probe results (30 seconds).
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// XMPP channel plugin.
pub struct XmppPlugin {
    accounts: AccountStateMap,
    outbound: XmppOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    probe_cache: RwLock<HashMap<String, (ChannelHealthSnapshot, Instant)>>,
}

impl XmppPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let outbound = XmppOutbound {
            accounts: Arc::clone(&accounts),
        };
        Self {
            accounts,
            outbound,
            message_log: None,
            event_sink: None,
            probe_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_message_log(mut self, log: Arc<dyn MessageLog>) -> Self {
        self.message_log = Some(log);
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn ChannelEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }
}

impl Default for XmppPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for XmppPlugin {
    fn id(&self) -> &str {
        "xmpp"
    }

    fn name(&self) -> &str {
        "XMPP"
    }

    async fn start_account(&mut self, account_id: &str, config: serde_json::Value) -> Result<()> {
        let xmpp_config: XmppAccountConfig = serde_json::from_value(config)?;

        if xmpp_config.jid.is_empty() {
            return Err(anyhow::anyhow!("XMPP JID is required"));
        }
        if xmpp_config.password.expose_secret().is_empty() {
            return Err(anyhow::anyhow!("XMPP password is required"));
        }

        info!(account_id, jid = %xmpp_config.jid, "starting xmpp account");

        client::start_event_loop(
            account_id.to_string(),
            xmpp_config,
            Arc::clone(&self.accounts),
            self.message_log.clone(),
            self.event_sink.clone(),
        )
        .await?;

        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> Result<()> {
        let cancel = {
            let accounts = self.accounts.read().await;
            accounts.get(account_id).map(|s| s.cancel.clone())
        };

        if let Some(cancel) = cancel {
            info!(account_id, "stopping xmpp account");
            cancel.cancel();
            let mut accounts = self.accounts.write().await;
            accounts.remove(account_id);
        } else {
            warn!(account_id, "xmpp account not found");
        }

        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        Some(&self.outbound)
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(XmppOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn status(&self) -> Option<&dyn ChannelStatus> {
        Some(self)
    }

    fn account_ids(&self) -> Vec<String> {
        // Use try_read to avoid blocking; return empty if locked.
        match self.accounts.try_read() {
            Ok(accounts) => accounts.keys().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    fn account_config(&self, account_id: &str) -> Option<serde_json::Value> {
        match self.accounts.try_read() {
            Ok(accounts) => accounts
                .get(account_id)
                .and_then(|s| serde_json::to_value(&s.config).ok()),
            Err(_) => None,
        }
    }
}

#[async_trait]
impl ChannelStatus for XmppPlugin {
    async fn probe(&self, account_id: &str) -> Result<ChannelHealthSnapshot> {
        // Return cached result if fresh enough.
        if let Ok(cache) = self.probe_cache.read()
            && let Some((snap, ts)) = cache.get(account_id)
            && ts.elapsed() < PROBE_CACHE_TTL
        {
            return Ok(snap.clone());
        }

        let connected = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|s| s.connected.load(std::sync::atomic::Ordering::Relaxed))
        };

        let result = match connected {
            Some(true) => ChannelHealthSnapshot {
                connected: true,
                account_id: account_id.to_string(),
                details: Some("connected".into()),
            },
            Some(false) => ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("disconnected (reconnecting)".into()),
            },
            None => ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not started".into()),
            },
        };

        if let Ok(mut cache) = self.probe_cache.write() {
            cache.insert(account_id.to_string(), (result.clone(), Instant::now()));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_and_name() {
        let plugin = XmppPlugin::new();
        assert_eq!(plugin.id(), "xmpp");
        assert_eq!(plugin.name(), "XMPP");
    }

    #[test]
    fn default_has_empty_accounts() {
        let plugin = XmppPlugin::default();
        assert!(plugin.account_ids().is_empty());
    }

    #[test]
    fn outbound_is_some() {
        let plugin = XmppPlugin::new();
        assert!(plugin.outbound().is_some());
        assert!(plugin.status().is_some());
    }

    #[tokio::test]
    async fn start_rejects_empty_jid() {
        let mut plugin = XmppPlugin::new();
        let config = serde_json::json!({ "jid": "", "password": "pass" });
        let result = plugin.start_account("test", config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("JID is required"));
    }

    #[tokio::test]
    async fn start_rejects_empty_password() {
        let mut plugin = XmppPlugin::new();
        let config = serde_json::json!({ "jid": "bot@example.com", "password": "" });
        let result = plugin.start_account("test", config).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("password is required")
        );
    }

    #[tokio::test]
    async fn stop_nonexistent_account() {
        let mut plugin = XmppPlugin::new();
        // Should not error, just warn.
        let result = plugin.stop_account("nonexistent").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn probe_unknown_account() {
        let plugin = XmppPlugin::new();
        let snap = plugin.probe("unknown").await.unwrap();
        assert!(!snap.connected);
        assert_eq!(snap.details.as_deref(), Some("account not started"));
    }
}
