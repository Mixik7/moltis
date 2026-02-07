use std::sync::Arc;

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{error, info, warn},
};

#[cfg(feature = "whatsapp-web")]
use moltis_whatsapp::WhatsAppPlugin as WhatsAppWebPlugin;
#[cfg(feature = "whatsapp-business")]
use moltis_whatsapp_business::WhatsAppPlugin;
use {moltis_channels::ChannelPlugin, moltis_telegram::TelegramPlugin};

use {
    moltis_channels::{
        message_log::MessageLog,
        store::{ChannelStore, StoredChannel},
    },
    moltis_sessions::metadata::SqliteSessionMetadata,
};

use crate::services::{ChannelService, ServiceResult};

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Live channel service backed by `TelegramPlugin` and optionally `WhatsAppPlugin`.
pub struct LiveChannelService {
    telegram: Arc<RwLock<TelegramPlugin>>,
    #[cfg(feature = "whatsapp-business")]
    whatsapp: Arc<RwLock<WhatsAppPlugin>>,
    #[cfg(feature = "whatsapp-web")]
    whatsapp_web: Arc<RwLock<WhatsAppWebPlugin>>,
    store: Arc<dyn ChannelStore>,
    message_log: Arc<dyn MessageLog>,
    session_metadata: Arc<SqliteSessionMetadata>,
}

impl LiveChannelService {
    #[cfg(all(feature = "whatsapp-business", feature = "whatsapp-web"))]
    pub fn new(
        telegram: TelegramPlugin,
        whatsapp: WhatsAppPlugin,
        whatsapp_web: WhatsAppWebPlugin,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            telegram: Arc::new(RwLock::new(telegram)),
            whatsapp: Arc::new(RwLock::new(whatsapp)),
            whatsapp_web: Arc::new(RwLock::new(whatsapp_web)),
            store,
            message_log,
            session_metadata,
        }
    }

    #[cfg(all(feature = "whatsapp-business", not(feature = "whatsapp-web")))]
    pub fn new(
        telegram: TelegramPlugin,
        whatsapp: WhatsAppPlugin,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            telegram: Arc::new(RwLock::new(telegram)),
            whatsapp: Arc::new(RwLock::new(whatsapp)),
            store,
            message_log,
            session_metadata,
        }
    }

    #[cfg(all(not(feature = "whatsapp-business"), feature = "whatsapp-web"))]
    pub fn new(
        telegram: TelegramPlugin,
        whatsapp_web: WhatsAppWebPlugin,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            telegram: Arc::new(RwLock::new(telegram)),
            whatsapp_web: Arc::new(RwLock::new(whatsapp_web)),
            store,
            message_log,
            session_metadata,
        }
    }

    #[cfg(all(not(feature = "whatsapp-business"), not(feature = "whatsapp-web")))]
    pub fn new(
        telegram: TelegramPlugin,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            telegram: Arc::new(RwLock::new(telegram)),
            store,
            message_log,
            session_metadata,
        }
    }

    /// Get a reference to the WhatsApp Business plugin (for webhook handlers).
    #[cfg(feature = "whatsapp-business")]
    pub fn whatsapp(&self) -> Arc<RwLock<WhatsAppPlugin>> {
        Arc::clone(&self.whatsapp)
    }

    /// Get a reference to the WhatsApp Web plugin.
    #[cfg(feature = "whatsapp-web")]
    pub fn whatsapp_web(&self) -> Arc<RwLock<WhatsAppWebPlugin>> {
        Arc::clone(&self.whatsapp_web)
    }

    /// Get allowlist from the stored channel config.
    async fn get_allowlist(&self, account_id: &str) -> Vec<String> {
        // Try to read allowlist from the store (works for any channel type).
        if let Ok(Some(stored)) = self.store.get(account_id).await
            && let Some(list) = stored
                .config
                .get("allowlist")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
        {
            return list;
        }

        // Fallback: try telegram config.
        let tg = self.telegram.read().await;
        if let Some(list) = tg
            .account_config(account_id)
            .and_then(|cfg| cfg.get("allowlist").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
        {
            return list;
        }
        drop(tg);

        // Fallback: try whatsapp business config.
        #[cfg(feature = "whatsapp-business")]
        {
            let wa = self.whatsapp.read().await;
            if let Some(list) = wa
                .account_config(account_id)
                .and_then(|cfg| cfg.get("allowlist").cloned())
                .and_then(|v| serde_json::from_value(v).ok())
            {
                return list;
            }
        }

        // Fallback: try whatsapp web config.
        #[cfg(feature = "whatsapp-web")]
        {
            let wa_web = self.whatsapp_web.read().await;
            if let Some(list) = wa_web
                .account_config(account_id)
                .and_then(|cfg| cfg.get("allowlist").cloned())
                .and_then(|v| serde_json::from_value(v).ok())
            {
                return list;
            }
        }

        Vec::new()
    }
}

#[async_trait]
impl ChannelService for LiveChannelService {
    async fn status(&self) -> ServiceResult {
        let mut channels = Vec::new();

        // Telegram channels
        {
            let tg = self.telegram.read().await;
            let account_ids = tg.account_ids();

            if let Some(status) = tg.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": "telegram",
                                "name": format!("Telegram ({})", aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = tg.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            // Include bound sessions and active session mappings.
                            let bound = self
                                .session_metadata
                                .list_account_sessions("telegram", aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions("telegram", aid)
                                .await;
                            let sessions: Vec<_> = bound
                                .iter()
                                .map(|s| {
                                    let is_active = active_map.iter().any(|(_, sk)| sk == &s.key);
                                    serde_json::json!({
                                        "key": s.key,
                                        "label": s.label,
                                        "messageCount": s.message_count,
                                        "active": is_active,
                                    })
                                })
                                .collect();
                            if !sessions.is_empty() {
                                entry["sessions"] = serde_json::json!(sessions);
                            }

                            channels.push(entry);
                        },
                        Err(e) => {
                            channels.push(serde_json::json!({
                                "type": "telegram",
                                "name": format!("Telegram ({})", aid),
                                "account_id": aid,
                                "status": "error",
                                "details": e.to_string(),
                            }));
                        },
                    }
                }
            }
        }

        // WhatsApp Business channels
        #[cfg(feature = "whatsapp-business")]
        {
            let wa = self.whatsapp.read().await;
            let account_ids = wa.account_ids();

            if let Some(status) = wa.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": "whatsapp",
                                "name": format!("WhatsApp Business ({})", aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = wa.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            // Include bound sessions and active session mappings.
                            let bound = self
                                .session_metadata
                                .list_account_sessions("whatsapp", aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions("whatsapp", aid)
                                .await;
                            let sessions: Vec<_> = bound
                                .iter()
                                .map(|s| {
                                    let is_active = active_map.iter().any(|(_, sk)| sk == &s.key);
                                    serde_json::json!({
                                        "key": s.key,
                                        "label": s.label,
                                        "messageCount": s.message_count,
                                        "active": is_active,
                                    })
                                })
                                .collect();
                            if !sessions.is_empty() {
                                entry["sessions"] = serde_json::json!(sessions);
                            }

                            channels.push(entry);
                        },
                        Err(e) => {
                            channels.push(serde_json::json!({
                                "type": "whatsapp",
                                "name": format!("WhatsApp Business ({})", aid),
                                "account_id": aid,
                                "status": "error",
                                "details": e.to_string(),
                            }));
                        },
                    }
                }
            }
        }

        // WhatsApp Web channels
        #[cfg(feature = "whatsapp-web")]
        {
            let wa_web = self.whatsapp_web.read().await;
            let account_ids = wa_web.account_ids();

            if let Some(status) = wa_web.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": "whatsapp-web",
                                "name": format!("WhatsApp Web ({})", aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = wa_web.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            // Include QR code if available.
                            if let Some(qr) = wa_web.get_qr_code(aid) {
                                entry["qr_code"] = serde_json::json!(qr);
                            }

                            // Include bound sessions and active session mappings.
                            let bound = self
                                .session_metadata
                                .list_account_sessions("whatsapp-web", aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions("whatsapp-web", aid)
                                .await;
                            let sessions: Vec<_> = bound
                                .iter()
                                .map(|s| {
                                    let is_active = active_map.iter().any(|(_, sk)| sk == &s.key);
                                    serde_json::json!({
                                        "key": s.key,
                                        "label": s.label,
                                        "messageCount": s.message_count,
                                        "active": is_active,
                                    })
                                })
                                .collect();
                            if !sessions.is_empty() {
                                entry["sessions"] = serde_json::json!(sessions);
                            }

                            channels.push(entry);
                        },
                        Err(e) => {
                            channels.push(serde_json::json!({
                                "type": "whatsapp-web",
                                "name": format!("WhatsApp Web ({})", aid),
                                "account_id": aid,
                                "status": "error",
                                "details": e.to_string(),
                            }));
                        },
                    }
                }
            }
        }

        Ok(serde_json::json!({ "channels": channels }))
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let config = params
            .get("config")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        match channel_type {
            "telegram" => {
                info!(account_id, "adding telegram channel account");

                let mut tg = self.telegram.write().await;
                tg.start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to start telegram account");
                        e.to_string()
                    })?;
            },
            #[cfg(feature = "whatsapp-business")]
            "whatsapp" => {
                info!(account_id, "adding whatsapp business channel account");

                let mut wa = self.whatsapp.write().await;
                wa.start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to start whatsapp business account");
                        e.to_string()
                    })?;
            },
            #[cfg(feature = "whatsapp-web")]
            "whatsapp-web" => {
                info!(account_id, "adding whatsapp web channel account");

                let mut wa_web = self.whatsapp_web.write().await;
                wa_web
                    .start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to start whatsapp web account");
                        e.to_string()
                    })?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}")),
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.into(),
                config,
                created_at: now,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel");
        }

        Ok(serde_json::json!({ "added": account_id }))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        match channel_type {
            "telegram" => {
                info!(account_id, "removing telegram channel account");
                let mut tg = self.telegram.write().await;
                tg.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop telegram account");
                    e.to_string()
                })?;
            },
            #[cfg(feature = "whatsapp-business")]
            "whatsapp" => {
                info!(account_id, "removing whatsapp business channel account");
                let mut wa = self.whatsapp.write().await;
                wa.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop whatsapp business account");
                    e.to_string()
                })?;
            },
            #[cfg(feature = "whatsapp-web")]
            "whatsapp-web" => {
                info!(account_id, "removing whatsapp web channel account");
                let mut wa_web = self.whatsapp_web.write().await;
                wa_web.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop whatsapp web account");
                    e.to_string()
                })?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}")),
        }

        if let Err(e) = self.store.delete(account_id).await {
            warn!(error = %e, account_id, "failed to delete channel from store");
        }

        Ok(serde_json::json!({ "removed": account_id }))
    }

    async fn logout(&self, params: Value) -> ServiceResult {
        self.remove(params).await
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let config = params
            .get("config")
            .cloned()
            .ok_or_else(|| "missing 'config'".to_string())?;

        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        match channel_type {
            "telegram" => {
                info!(account_id, "updating telegram channel account");
                let mut tg = self.telegram.write().await;

                // Stop then restart with new config
                tg.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop telegram account for update");
                    e.to_string()
                })?;

                tg.start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to restart telegram account after update");
                        e.to_string()
                    })?;
            },
            #[cfg(feature = "whatsapp-business")]
            "whatsapp" => {
                info!(account_id, "updating whatsapp business channel account");
                let mut wa = self.whatsapp.write().await;

                // Stop then restart with new config
                wa.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop whatsapp business account for update");
                    e.to_string()
                })?;

                wa.start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to restart whatsapp business account after update");
                        e.to_string()
                    })?;
            },
            #[cfg(feature = "whatsapp-web")]
            "whatsapp-web" => {
                info!(account_id, "updating whatsapp web channel account");
                let mut wa_web = self.whatsapp_web.write().await;

                // Stop then restart with new config
                wa_web.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop whatsapp web account for update");
                    e.to_string()
                })?;

                wa_web
                    .start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to restart whatsapp web account after update");
                        e.to_string()
                    })?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}")),
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.into(),
                config,
                created_at: now,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel update");
        }

        Ok(serde_json::json!({ "updated": account_id }))
    }

    async fn send(&self, _params: Value) -> ServiceResult {
        Err("direct channel send not yet implemented".into())
    }

    async fn senders_list(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let senders = self
            .message_log
            .unique_senders(account_id)
            .await
            .map_err(|e| e.to_string())?;

        let allowlist = self.get_allowlist(account_id).await;

        // Query pending OTP challenges for this account.
        let otp_challenges = {
            let tg_inner = self.telegram.read().await;
            tg_inner.pending_otp_challenges(account_id)
        };

        let list: Vec<Value> = senders
            .into_iter()
            .map(|s| {
                let is_allowed = allowlist.iter().any(|a| {
                    let a_lower = a.to_lowercase();
                    a_lower == s.peer_id.to_lowercase()
                        || s.username
                            .as_ref()
                            .is_some_and(|u| a_lower == u.to_lowercase())
                });
                let mut entry = serde_json::json!({
                    "peer_id": s.peer_id,
                    "username": s.username,
                    "sender_name": s.sender_name,
                    "message_count": s.message_count,
                    "last_seen": s.last_seen,
                    "allowed": is_allowed,
                });
                // Attach OTP info if a challenge is pending for this peer.
                if let Some(otp) = otp_challenges.iter().find(|c| c.peer_id == s.peer_id) {
                    entry["otp_pending"] = serde_json::json!({
                        "code": otp.code,
                        "expires_at": otp.expires_at,
                    });
                }
                entry
            })
            .collect();

        Ok(serde_json::json!({ "senders": list }))
    }

    async fn sender_approve(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let identifier = params
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'identifier'".to_string())?;

        // Read current stored config, add identifier to allowlist, persist & restart.
        let stored = self
            .store
            .get(account_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("channel '{account_id}' not found in store"))?;

        let mut config = stored.config.clone();
        let allowlist = config
            .as_object_mut()
            .ok_or_else(|| "config is not an object".to_string())?
            .entry("allowlist")
            .or_insert_with(|| serde_json::json!([]));

        let arr = allowlist
            .as_array_mut()
            .ok_or_else(|| "allowlist is not an array".to_string())?;

        let id_lower = identifier.to_lowercase();
        if !arr
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.to_lowercase() == id_lower))
        {
            arr.push(serde_json::json!(identifier));
        }

        // Also ensure dm_policy is set to "allowlist" so the list is enforced.
        config
            .as_object_mut()
            .unwrap()
            .insert("dm_policy".into(), serde_json::json!("allowlist"));

        // Persist.
        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: stored.channel_type.clone(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender approval");
        }

        // Hot-update config for Telegram (preserves polling offset), restart for WhatsApp.
        match stored.channel_type.as_str() {
            "telegram" => {
                let tg = self.telegram.read().await;
                if let Err(e) = tg.update_account_config(account_id, config) {
                    warn!(error = %e, account_id, "failed to hot-update config for sender approval");
                }
            },
            _ => {
                self.restart_account(&stored.channel_type, account_id, config)
                    .await?;
            },
        }

        info!(account_id, identifier, "sender approved");
        Ok(serde_json::json!({ "approved": identifier }))
    }

    async fn sender_deny(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let identifier = params
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'identifier'".to_string())?;

        let stored = self
            .store
            .get(account_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("channel '{account_id}' not found in store"))?;

        let mut config = stored.config.clone();
        if let Some(arr) = config
            .as_object_mut()
            .and_then(|o| o.get_mut("allowlist"))
            .and_then(|v| v.as_array_mut())
        {
            let id_lower = identifier.to_lowercase();
            arr.retain(|v| v.as_str().is_none_or(|s| s.to_lowercase() != id_lower));
        }

        // Persist.
        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: stored.channel_type.clone(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender denial");
        }

        // Hot-update config for Telegram (no restart needed), restart for WhatsApp.
        match stored.channel_type.as_str() {
            "telegram" => {
                let tg = self.telegram.read().await;
                if let Err(e) = tg.update_account_config(account_id, config) {
                    warn!(error = %e, account_id, "failed to hot-update config for sender denial");
                }
            },
            _ => {
                self.restart_account(&stored.channel_type, account_id, config)
                    .await?;
            },
        }

        info!(account_id, identifier, "sender denied");
        Ok(serde_json::json!({ "denied": identifier }))
    }
}

impl LiveChannelService {
    /// Restart an account with new config.
    async fn restart_account(
        &self,
        channel_type: &str,
        account_id: &str,
        config: Value,
    ) -> Result<(), String> {
        match channel_type {
            "telegram" => {
                let mut tg = self.telegram.write().await;
                if let Err(e) = tg.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop telegram account");
                }
                tg.start_account(account_id, config)
                    .await
                    .map_err(|e| e.to_string())?;
            },
            #[cfg(feature = "whatsapp-business")]
            "whatsapp" => {
                let mut wa = self.whatsapp.write().await;
                if let Err(e) = wa.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop whatsapp business account");
                }
                wa.start_account(account_id, config)
                    .await
                    .map_err(|e| e.to_string())?;
            },
            #[cfg(feature = "whatsapp-web")]
            "whatsapp-web" => {
                let mut wa_web = self.whatsapp_web.write().await;
                if let Err(e) = wa_web.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop whatsapp web account");
                }
                wa_web
                    .start_account(account_id, config)
                    .await
                    .map_err(|e| e.to_string())?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}")),
        }
        Ok(())
    }
}
