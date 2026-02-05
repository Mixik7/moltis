use std::sync::Arc;

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{error, info, warn},
};

use moltis_channels::registry::ChannelRegistry;

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

/// Live channel service backed by `ChannelRegistry` (supports multiple channel types).
pub struct LiveChannelService {
    registry: Arc<RwLock<ChannelRegistry>>,
    store: Arc<dyn ChannelStore>,
    message_log: Arc<dyn MessageLog>,
    session_metadata: Arc<SqliteSessionMetadata>,
}

impl LiveChannelService {
    pub fn new(
        registry: ChannelRegistry,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            registry: Arc::new(RwLock::new(registry)),
            store,
            message_log,
            session_metadata,
        }
    }
}

#[async_trait]
impl ChannelService for LiveChannelService {
    async fn status(&self) -> ServiceResult {
        let reg = self.registry.read().await;
        let mut channels = Vec::new();

        for plugin_id in reg.list() {
            let plugin = match reg.get(plugin_id) {
                Some(p) => p,
                None => continue,
            };
            let channel_type = plugin.id();
            let channel_name = plugin.name();
            let account_ids = plugin.account_ids();

            if let Some(status) = plugin.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": channel_type,
                                "name": format!("{} ({})", channel_name, aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = plugin.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            // Include bound sessions and active session mappings.
                            let bound = self
                                .session_metadata
                                .list_account_sessions(channel_type, aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions(channel_type, aid)
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
                                "type": channel_type,
                                "name": format!("{} ({})", channel_name, aid),
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
            .ok_or_else(|| "missing 'type'".to_string())?;

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let config = params
            .get("config")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        info!(account_id, channel_type, "adding channel account");

        let mut reg = self.registry.write().await;
        let plugin = reg
            .get_mut(channel_type)
            .ok_or_else(|| format!("unsupported channel type: {channel_type}"))?;
        plugin
            .start_account(account_id, config.clone())
            .await
            .map_err(|e| {
                error!(error = %e, account_id, channel_type, "failed to start account");
                e.to_string()
            })?;

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.to_string(),
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

        // Look up the channel type from the store so we know which plugin to stop.
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let channel_type = if let Some(ct) = channel_type {
            ct
        } else {
            // Find which plugin owns this account_id.
            let reg = self.registry.read().await;
            let mut found = None;
            for pid in reg.list() {
                if let Some(p) = reg.get(pid)
                    && p.account_ids().contains(&account_id.to_string())
                {
                    found = Some(pid.to_string());
                    break;
                }
            }
            drop(reg);
            found.ok_or_else(|| format!("account '{account_id}' not found in any plugin"))?
        };

        info!(account_id, channel_type = %channel_type, "removing channel account");

        let mut reg = self.registry.write().await;
        if let Some(plugin) = reg.get_mut(&channel_type) {
            plugin.stop_account(account_id).await.map_err(|e| {
                error!(error = %e, account_id, "failed to stop account");
                e.to_string()
            })?;
        }
        drop(reg);

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
            .map(|s| s.to_string());

        let channel_type = if let Some(ct) = channel_type {
            ct
        } else {
            let reg = self.registry.read().await;
            let mut found = None;
            for pid in reg.list() {
                if let Some(p) = reg.get(pid)
                    && p.account_ids().contains(&account_id.to_string())
                {
                    found = Some(pid.to_string());
                    break;
                }
            }
            drop(reg);
            found.ok_or_else(|| format!("account '{account_id}' not found in any plugin"))?
        };

        info!(account_id, channel_type = %channel_type, "updating channel account");

        let mut reg = self.registry.write().await;
        let plugin = reg
            .get_mut(&channel_type)
            .ok_or_else(|| format!("unsupported channel type: {channel_type}"))?;

        // Stop then restart with new config
        plugin.stop_account(account_id).await.map_err(|e| {
            error!(error = %e, account_id, "failed to stop account for update");
            e.to_string()
        })?;

        plugin
            .start_account(account_id, config.clone())
            .await
            .map_err(|e| {
                error!(error = %e, account_id, "failed to restart account after update");
                e.to_string()
            })?;
        drop(reg);

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type,
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

        // Read allowlist from current config to tag each sender.
        // Find the plugin that owns this account.
        let reg = self.registry.read().await;
        let allowlist: Vec<String> = reg
            .list()
            .iter()
            .find_map(|pid| {
                reg.get(pid).and_then(|p| {
                    if p.account_ids().contains(&account_id.to_string()) {
                        p.account_config(account_id)
                    } else {
                        None
                    }
                })
            })
            .and_then(|cfg| cfg.get("allowlist").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        drop(reg);

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
                serde_json::json!({
                    "peer_id": s.peer_id,
                    "username": s.username,
                    "sender_name": s.sender_name,
                    "message_count": s.message_count,
                    "last_seen": s.last_seen,
                    "allowed": is_allowed,
                })
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

        let channel_type = stored.channel_type.clone();

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
                channel_type: channel_type.clone(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender approval");
        }

        // Restart account with new config.
        let mut reg = self.registry.write().await;
        if let Some(plugin) = reg.get_mut(&channel_type) {
            if let Err(e) = plugin.stop_account(account_id).await {
                warn!(error = %e, account_id, "failed to stop account for sender approval");
            }
            plugin
                .start_account(account_id, config)
                .await
                .map_err(|e| e.to_string())?;
        }
        drop(reg);

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

        let channel_type = stored.channel_type.clone();

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
                channel_type: channel_type.clone(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender denial");
        }

        // Restart account with new config.
        let mut reg = self.registry.write().await;
        if let Some(plugin) = reg.get_mut(&channel_type) {
            if let Err(e) = plugin.stop_account(account_id).await {
                warn!(error = %e, account_id, "failed to stop account for sender denial");
            }
            plugin
                .start_account(account_id, config)
                .await
                .map_err(|e| e.to_string())?;
        }
        drop(reg);

        info!(account_id, identifier, "sender denied");
        Ok(serde_json::json!({ "denied": identifier }))
    }
}
