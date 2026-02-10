use std::{collections::HashMap, sync::Arc};

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{error, info, warn},
};

use {
    moltis_channels::{
        ChannelPlugin, ChannelType,
        message_log::MessageLog,
        plugin::ChannelOutbound,
        store::{ChannelStore, StoredChannel},
    },
    moltis_sessions::metadata::SqliteSessionMetadata,
    moltis_telegram::TelegramPlugin,
};

use crate::services::{ChannelService, ServiceResult};

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Multi-channel service supporting Telegram and WhatsApp.
///
/// Each plugin type is stored as a concrete field behind its feature flag.
/// Telegram-specific features (OTP, allowlist hot-update) use the direct
/// `telegram` reference. The `account_types` reverse map is shared with
/// `MultiChannelOutbound` for routing.
pub struct LiveChannelService {
    store: Arc<dyn ChannelStore>,
    message_log: Arc<dyn MessageLog>,
    session_metadata: Arc<SqliteSessionMetadata>,
    /// Reverse map: account_id → ChannelType. Shared with `MultiChannelOutbound`.
    account_types: Arc<RwLock<HashMap<String, ChannelType>>>,
    /// Direct reference to the Telegram plugin.
    telegram: Option<Arc<RwLock<TelegramPlugin>>>,
    /// Direct reference to the WhatsApp plugin.
    #[cfg(feature = "whatsapp")]
    whatsapp: Option<Arc<RwLock<moltis_whatsapp::WhatsAppPlugin>>>,
}

impl LiveChannelService {
    pub fn new(
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            store,
            message_log,
            session_metadata,
            account_types: Arc::new(RwLock::new(HashMap::new())),
            telegram: None,
            #[cfg(feature = "whatsapp")]
            whatsapp: None,
        }
    }

    /// Register a Telegram plugin.
    pub fn register_telegram(&mut self, plugin: TelegramPlugin) {
        self.telegram = Some(Arc::new(RwLock::new(plugin)));
    }

    /// Register a WhatsApp plugin.
    #[cfg(feature = "whatsapp")]
    pub fn register_whatsapp(&mut self, plugin: moltis_whatsapp::WhatsAppPlugin) {
        self.whatsapp = Some(Arc::new(RwLock::new(plugin)));
    }

    /// Get a shared reference to the account_types map (for `MultiChannelOutbound`).
    pub fn account_types(&self) -> Arc<RwLock<HashMap<String, ChannelType>>> {
        Arc::clone(&self.account_types)
    }

    /// Resolve account_id → ChannelType from the in-memory reverse map,
    /// falling back to the persistent store.
    async fn resolve_type(&self, account_id: &str) -> Option<ChannelType> {
        {
            let map = self.account_types.read().await;
            if let Some(ct) = map.get(account_id) {
                return Some(*ct);
            }
        }
        // Fall back to store.
        if let Ok(Some(stored)) = self.store.get(account_id).await
            && let Ok(ct) = stored.channel_type.parse::<ChannelType>()
        {
            let mut map = self.account_types.write().await;
            map.insert(account_id.to_string(), ct);
            return Some(ct);
        }
        None
    }

    /// Record an account → type mapping.
    async fn track_account(&self, account_id: &str, ct: ChannelType) {
        let mut map = self.account_types.write().await;
        map.insert(account_id.to_string(), ct);
    }

    /// Remove an account → type mapping.
    async fn untrack_account(&self, account_id: &str) {
        let mut map = self.account_types.write().await;
        map.remove(account_id);
    }

    /// Helper: build session info for an account.
    async fn session_info(&self, ct_str: &str, account_id: &str) -> Vec<serde_json::Value> {
        let bound = self
            .session_metadata
            .list_account_sessions(ct_str, account_id)
            .await;
        let active_map = self
            .session_metadata
            .list_active_sessions(ct_str, account_id)
            .await;
        bound
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
            .collect()
    }

    /// Collect Telegram channel status entries.
    async fn telegram_status(&self) -> Vec<serde_json::Value> {
        let mut channels = Vec::new();
        let Some(ref tg_arc) = self.telegram else {
            return channels;
        };
        let tg = tg_arc.read().await;
        let account_ids = tg.account_ids();
        let Some(status) = tg.status() else {
            return channels;
        };

        let ct_str = ChannelType::Telegram.as_str();
        for aid in &account_ids {
            match status.probe(aid).await {
                Ok(snap) => {
                    let mut entry = serde_json::json!({
                        "type": ct_str,
                        "name": format!("Telegram ({aid})"),
                        "account_id": aid,
                        "status": if snap.connected { "connected" } else { "disconnected" },
                        "details": snap.details,
                    });
                    if let Some(cfg) = tg.account_config(aid) {
                        entry["config"] = cfg;
                    }
                    let sessions = self.session_info(ct_str, aid).await;
                    if !sessions.is_empty() {
                        entry["sessions"] = serde_json::json!(sessions);
                    }
                    channels.push(entry);
                },
                Err(e) => {
                    channels.push(serde_json::json!({
                        "type": ct_str,
                        "name": format!("Telegram ({aid})"),
                        "account_id": aid,
                        "status": "error",
                        "details": e.to_string(),
                    }));
                },
            }
        }
        channels
    }

    /// Collect WhatsApp channel status entries.
    #[cfg(feature = "whatsapp")]
    async fn whatsapp_status(&self) -> Vec<serde_json::Value> {
        let mut channels = Vec::new();
        let Some(ref wa_arc) = self.whatsapp else {
            return channels;
        };
        let wa = wa_arc.read().await;
        let account_ids = wa.account_ids();
        let Some(status) = wa.status() else {
            return channels;
        };

        let ct_str = ChannelType::Whatsapp.as_str();
        for aid in &account_ids {
            match status.probe(aid).await {
                Ok(snap) => {
                    let qr = wa.latest_qr(aid);
                    let status_str = if snap.connected {
                        "connected"
                    } else if qr.is_some() {
                        "pairing"
                    } else {
                        "disconnected"
                    };
                    let mut entry = serde_json::json!({
                        "type": ct_str,
                        "name": format!("WhatsApp ({aid})"),
                        "account_id": aid,
                        "status": status_str,
                        "details": snap.details,
                    });
                    if let Some(ref qr_data) = qr {
                        entry["qr_data"] = serde_json::json!(qr_data);
                    }
                    if let Some(cfg) = wa.account_config(aid) {
                        entry["config"] = cfg;
                    }
                    let sessions = self.session_info(ct_str, aid).await;
                    if !sessions.is_empty() {
                        entry["sessions"] = serde_json::json!(sessions);
                    }
                    channels.push(entry);
                },
                Err(e) => {
                    channels.push(serde_json::json!({
                        "type": ct_str,
                        "name": format!("WhatsApp ({aid})"),
                        "account_id": aid,
                        "status": "error",
                        "details": e.to_string(),
                    }));
                },
            }
        }
        channels
    }
}

#[async_trait]
impl ChannelService for LiveChannelService {
    async fn status(&self) -> ServiceResult {
        let mut channels = self.telegram_status().await;
        #[cfg(feature = "whatsapp")]
        channels.extend(self.whatsapp_status().await);
        Ok(serde_json::json!({ "channels": channels }))
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let channel_type_str = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let ct: ChannelType = channel_type_str
            .parse()
            .map_err(|_| format!("unsupported channel type: {channel_type_str}"))?;

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let config = params
            .get("config")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        info!(account_id, channel_type = %ct, "adding channel account");

        match ct {
            ChannelType::Telegram => {
                if let Some(ref tg_arc) = self.telegram {
                    let mut tg = tg_arc.write().await;
                    tg.start_account(account_id, config.clone())
                        .await
                        .map_err(|e| {
                            error!(error = %e, account_id, "failed to start telegram account");
                            e.to_string()
                        })?;
                } else {
                    return Err("telegram plugin not registered".into());
                }
            },
            #[cfg(feature = "whatsapp")]
            ChannelType::Whatsapp => {
                if let Some(ref wa_arc) = self.whatsapp {
                    let mut wa = wa_arc.write().await;
                    wa.start_account(account_id, config.clone())
                        .await
                        .map_err(|e| {
                            error!(error = %e, account_id, "failed to start whatsapp account");
                            e.to_string()
                        })?;
                } else {
                    return Err("whatsapp plugin not registered".into());
                }
            },
            #[allow(unreachable_patterns)]
            _ => return Err(format!("unsupported channel type: {ct}")),
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: ct.to_string(),
                config,
                created_at: now,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel");
        }

        self.track_account(account_id, ct).await;

        Ok(serde_json::json!({ "added": account_id }))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let ct = self
            .resolve_type(account_id)
            .await
            .ok_or_else(|| format!("unknown account: {account_id}"))?;

        info!(account_id, channel_type = %ct, "removing channel account");

        match ct {
            ChannelType::Telegram => {
                if let Some(ref tg_arc) = self.telegram {
                    let mut tg = tg_arc.write().await;
                    tg.stop_account(account_id).await.map_err(|e| {
                        error!(error = %e, account_id, "failed to stop telegram account");
                        e.to_string()
                    })?;
                }
            },
            #[cfg(feature = "whatsapp")]
            ChannelType::Whatsapp => {
                if let Some(ref wa_arc) = self.whatsapp {
                    let mut wa = wa_arc.write().await;
                    wa.stop_account(account_id).await.map_err(|e| {
                        error!(error = %e, account_id, "failed to stop whatsapp account");
                        e.to_string()
                    })?;
                }
            },
            #[allow(unreachable_patterns)]
            _ => {},
        }

        if let Err(e) = self.store.delete(account_id).await {
            warn!(error = %e, account_id, "failed to delete channel from store");
        }

        self.untrack_account(account_id).await;

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

        let ct = self
            .resolve_type(account_id)
            .await
            .ok_or_else(|| format!("unknown account: {account_id}"))?;

        info!(account_id, channel_type = %ct, "updating channel account");

        match ct {
            ChannelType::Telegram => {
                if let Some(ref tg_arc) = self.telegram {
                    let mut tg = tg_arc.write().await;
                    tg.stop_account(account_id).await.map_err(|e| {
                        error!(error = %e, account_id, "failed to stop telegram for update");
                        e.to_string()
                    })?;
                    tg.start_account(account_id, config.clone())
                        .await
                        .map_err(|e| {
                            error!(error = %e, account_id, "failed to restart telegram after update");
                            e.to_string()
                        })?;
                }
            },
            #[cfg(feature = "whatsapp")]
            ChannelType::Whatsapp => {
                if let Some(ref wa_arc) = self.whatsapp {
                    let mut wa = wa_arc.write().await;
                    wa.stop_account(account_id).await.map_err(|e| {
                        error!(error = %e, account_id, "failed to stop whatsapp for update");
                        e.to_string()
                    })?;
                    wa.start_account(account_id, config.clone())
                        .await
                        .map_err(|e| {
                            error!(error = %e, account_id, "failed to restart whatsapp after update");
                            e.to_string()
                        })?;
                }
            },
            #[allow(unreachable_patterns)]
            _ => return Err(format!("unsupported channel type: {ct}")),
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: ct.to_string(),
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

        let ct = self.resolve_type(account_id).await;

        // Collect allowlist and OTP challenges (serialized to Value for type
        // uniformity across Telegram and WhatsApp plugin types).
        let (allowlist, otp_challenges): (Vec<String>, Vec<Value>) = match ct {
            Some(ChannelType::Telegram) => {
                if let Some(ref tg_arc) = self.telegram {
                    let tg = tg_arc.read().await;
                    let al: Vec<String> = tg
                        .account_config(account_id)
                        .and_then(|cfg| cfg.get("allowlist").cloned())
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    let otp: Vec<Value> = tg
                        .pending_otp_challenges(account_id)
                        .into_iter()
                        .filter_map(|c| serde_json::to_value(c).ok())
                        .collect();
                    (al, otp)
                } else {
                    (Vec::new(), Vec::new())
                }
            },
            #[cfg(feature = "whatsapp")]
            Some(ChannelType::Whatsapp) => {
                if let Some(ref wa_arc) = self.whatsapp {
                    let wa = wa_arc.read().await;
                    let al: Vec<String> = wa
                        .account_config(account_id)
                        .and_then(|cfg| cfg.get("allowlist").cloned())
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    let otp: Vec<Value> = wa
                        .pending_otp_challenges(account_id)
                        .into_iter()
                        .filter_map(|c| serde_json::to_value(c).ok())
                        .collect();
                    (al, otp)
                } else {
                    (Vec::new(), Vec::new())
                }
            },
            _ => (Vec::new(), Vec::new()),
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
                if let Some(otp) = otp_challenges.iter().find(|c| {
                    c.get("peer_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|pid| pid == s.peer_id)
                }) {
                    entry["otp_pending"] = otp.clone();
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

        let ct = self
            .resolve_type(account_id)
            .await
            .ok_or_else(|| format!("unknown account: {account_id}"))?;

        // Update allowlist and persist.
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
        if let Some(obj) = config.as_object_mut() {
            obj.insert("dm_policy".into(), serde_json::json!("allowlist"));
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: ct.to_string(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender approval");
        }

        // Hot-update the in-memory config for the correct plugin.
        match ct {
            ChannelType::Telegram => {
                if let Some(ref tg_arc) = self.telegram {
                    let tg = tg_arc.read().await;
                    if let Err(e) = tg.update_account_config(account_id, config) {
                        warn!(error = %e, account_id, "failed to hot-update telegram config");
                    }
                }
            },
            #[cfg(feature = "whatsapp")]
            ChannelType::Whatsapp => {
                if let Some(ref wa_arc) = self.whatsapp {
                    let wa = wa_arc.read().await;
                    if let Err(e) = wa.update_account_config(account_id, config) {
                        warn!(error = %e, account_id, "failed to hot-update whatsapp config");
                    }
                }
            },
            #[allow(unreachable_patterns)]
            _ => {},
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

        let ct = self
            .resolve_type(account_id)
            .await
            .ok_or_else(|| format!("unknown account: {account_id}"))?;

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

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: ct.to_string(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender denial");
        }

        // Hot-update the in-memory config for the correct plugin.
        match ct {
            ChannelType::Telegram => {
                if let Some(ref tg_arc) = self.telegram {
                    let tg = tg_arc.read().await;
                    if let Err(e) = tg.update_account_config(account_id, config) {
                        warn!(error = %e, account_id, "failed to hot-update telegram config");
                    }
                }
            },
            #[cfg(feature = "whatsapp")]
            ChannelType::Whatsapp => {
                if let Some(ref wa_arc) = self.whatsapp {
                    let wa = wa_arc.read().await;
                    if let Err(e) = wa.update_account_config(account_id, config) {
                        warn!(error = %e, account_id, "failed to hot-update whatsapp config");
                    }
                }
            },
            #[allow(unreachable_patterns)]
            _ => {},
        }

        info!(account_id, identifier, "sender denied");
        Ok(serde_json::json!({ "denied": identifier }))
    }
}

/// Multi-channel outbound that routes send operations to the correct plugin
/// based on the account_id → ChannelType mapping.
pub struct MultiChannelOutbound {
    telegram_outbound: Option<Arc<dyn ChannelOutbound>>,
    #[cfg(feature = "whatsapp")]
    whatsapp_outbound: Option<Arc<dyn ChannelOutbound>>,
    account_types: Arc<RwLock<HashMap<String, ChannelType>>>,
}

impl MultiChannelOutbound {
    pub fn new(account_types: Arc<RwLock<HashMap<String, ChannelType>>>) -> Self {
        Self {
            telegram_outbound: None,
            #[cfg(feature = "whatsapp")]
            whatsapp_outbound: None,
            account_types,
        }
    }

    pub fn with_telegram(mut self, outbound: Arc<dyn ChannelOutbound>) -> Self {
        self.telegram_outbound = Some(outbound);
        self
    }

    #[cfg(feature = "whatsapp")]
    pub fn with_whatsapp(mut self, outbound: Arc<dyn ChannelOutbound>) -> Self {
        self.whatsapp_outbound = Some(outbound);
        self
    }

    async fn resolve(&self, account_id: &str) -> Option<Arc<dyn ChannelOutbound>> {
        let map = self.account_types.read().await;
        match map.get(account_id) {
            Some(ChannelType::Telegram) => self.telegram_outbound.clone(),
            #[cfg(feature = "whatsapp")]
            Some(ChannelType::Whatsapp) => self.whatsapp_outbound.clone(),
            _ => self.telegram_outbound.clone(), // default fallback
        }
    }
}

#[async_trait]
impl ChannelOutbound for MultiChannelOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(ob) = self.resolve(account_id).await {
            ob.send_text(account_id, to, text, reply_to).await
        } else {
            Err(anyhow::anyhow!("no outbound for account: {account_id}"))
        }
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &moltis_common::types::ReplyPayload,
        reply_to: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(ob) = self.resolve(account_id).await {
            ob.send_media(account_id, to, payload, reply_to).await
        } else {
            Err(anyhow::anyhow!("no outbound for account: {account_id}"))
        }
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> anyhow::Result<()> {
        if let Some(ob) = self.resolve(account_id).await {
            ob.send_typing(account_id, to).await
        } else {
            Ok(()) // typing is best-effort
        }
    }
}
