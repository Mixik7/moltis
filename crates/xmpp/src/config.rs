use std::collections::HashMap;

use {
    moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

/// Actions the XMPP plugin can perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XmppActions {
    /// Whether to send XEP-0444 emoji reactions.
    pub reactions: bool,
}

impl Default for XmppActions {
    fn default() -> Self {
        Self { reactions: true }
    }
}

/// Per-MUC room configuration overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MucRoomConfig {
    /// Whether this room is enabled.
    pub enabled: bool,
    /// Override require_mention for this room.
    pub require_mention: Option<bool>,
    /// Per-room user allowlist (JID patterns).
    pub users: Vec<String>,
    /// Custom system prompt for this room.
    pub system_prompt: Option<String>,
    /// Skill overrides for this room.
    pub skills: Vec<String>,
}

impl Default for MucRoomConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            require_mention: None,
            users: Vec::new(),
            system_prompt: None,
            skills: Vec::new(),
        }
    }
}

/// Configuration for a single XMPP account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XmppAccountConfig {
    /// User JID (e.g. "bot@example.com").
    pub jid: String,

    /// Account password.
    #[serde(serialize_with = "serialize_secret")]
    pub password: Secret<String>,

    /// Optional TCP host or wss:// URL override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,

    /// XMPP resource (default "moltis").
    pub resource: String,

    /// MUC rooms to auto-join on connect (full room JIDs).
    pub rooms: Vec<String>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group/MUC access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for MUC rooms.
    pub mention_mode: MentionMode,

    /// User/JID allowlist for DMs (supports `*@domain.com` globs).
    pub allowlist: Vec<String>,

    /// Group/room JID allowlist.
    pub group_allowlist: Vec<String>,

    /// Per-room configuration overrides, keyed by room JID.
    pub muc_rooms: HashMap<String, MucRoomConfig>,

    /// XMPP-specific actions.
    pub actions: XmppActions,

    /// Maximum characters per text message chunk (XMPP has no hard limit,
    /// but very long messages are unwieldy).
    pub text_chunk_limit: usize,

    /// Maximum media file size in MB for HTTP Upload.
    pub media_max_mb: u32,

    /// Media MIME types to block.
    pub blocked_media_types: Vec<String>,

    /// Default model ID for this account's sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

impl std::fmt::Debug for XmppAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XmppAccountConfig")
            .field("jid", &self.jid)
            .field("password", &"[REDACTED]")
            .field("resource", &self.resource)
            .field("rooms", &self.rooms)
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .finish_non_exhaustive()
    }
}

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

impl Default for XmppAccountConfig {
    fn default() -> Self {
        Self {
            jid: String::new(),
            password: Secret::new(String::new()),
            server: None,
            resource: "moltis".into(),
            rooms: Vec::new(),
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            mention_mode: MentionMode::default(),
            allowlist: Vec::new(),
            group_allowlist: Vec::new(),
            muc_rooms: HashMap::new(),
            actions: XmppActions::default(),
            text_chunk_limit: 4000,
            media_max_mb: 20,
            blocked_media_types: Vec::new(),
            model: None,
            model_provider: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = XmppAccountConfig::default();
        assert_eq!(cfg.resource, "moltis");
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.text_chunk_limit, 4000);
        assert_eq!(cfg.media_max_mb, 20);
    }

    #[test]
    fn deserialize_from_json() {
        let json = r#"{
            "jid": "bot@example.com",
            "password": "secret123",
            "rooms": ["room1@conference.example.com"],
            "dm_policy": "allowlist",
            "allowlist": ["alice@example.com", "*@trusted.org"]
        }"#;
        let cfg: XmppAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.jid, "bot@example.com");
        assert_eq!(cfg.password.expose_secret(), "secret123");
        assert_eq!(cfg.rooms, vec!["room1@conference.example.com"]);
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.allowlist.len(), 2);
        // defaults for unspecified fields
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert_eq!(cfg.resource, "moltis");
    }

    #[test]
    fn serialize_roundtrip() {
        let cfg = XmppAccountConfig {
            jid: "bot@example.com".into(),
            password: Secret::new("pass".into()),
            dm_policy: DmPolicy::Disabled,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg2: XmppAccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.dm_policy, DmPolicy::Disabled);
        assert_eq!(cfg2.password.expose_secret(), "pass");
    }

    #[test]
    fn muc_room_config_defaults() {
        let room = MucRoomConfig::default();
        assert!(room.enabled);
        assert!(room.require_mention.is_none());
        assert!(room.users.is_empty());
        assert!(room.system_prompt.is_none());
    }
}
