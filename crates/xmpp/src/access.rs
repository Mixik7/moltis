//! XMPP access control.
//!
//! Determines whether an inbound message should be processed based on
//! DM/group policies, allowlists, mention mode, and per-room overrides.

use {
    moltis_channels::gating::{self, DmPolicy, GroupPolicy, MentionMode},
    moltis_common::types::ChatType,
};

use crate::config::XmppAccountConfig;

/// Determine if an inbound message should be processed.
///
/// Returns `Ok(())` if the message is allowed, or `Err(reason)` if it should
/// be silently dropped.
pub fn check_access(
    config: &XmppAccountConfig,
    chat_type: &ChatType,
    peer_jid: &str,
    room_jid: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match chat_type {
        ChatType::Dm => check_dm_access(config, peer_jid),
        ChatType::Group | ChatType::Channel => {
            check_group_access(config, peer_jid, room_jid, bot_mentioned)
        },
    }
}

fn check_dm_access(config: &XmppAccountConfig, peer_jid: &str) -> Result<(), AccessDenied> {
    match config.dm_policy {
        DmPolicy::Disabled => Err(AccessDenied::DmsDisabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
            if gating::is_allowed(peer_jid, &config.allowlist) {
                Ok(())
            } else {
                Err(AccessDenied::NotOnAllowlist)
            }
        },
    }
}

fn check_group_access(
    config: &XmppAccountConfig,
    peer_jid: &str,
    room_jid: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    // Group policy gate.
    match config.group_policy {
        GroupPolicy::Disabled => return Err(AccessDenied::GroupsDisabled),
        GroupPolicy::Allowlist => {
            let rid = room_jid.unwrap_or("");
            if !gating::is_allowed(rid, &config.group_allowlist) {
                return Err(AccessDenied::GroupNotOnAllowlist);
            }
        },
        GroupPolicy::Open => {},
    }

    // Per-room config: check if the room is disabled or has a user allowlist.
    if let Some(rid) = room_jid
        && let Some(room_config) = config.muc_rooms.get(rid)
    {
        if !room_config.enabled {
            return Err(AccessDenied::RoomDisabled);
        }

        // Per-room user allowlist.
        if !room_config.users.is_empty() && !gating::is_allowed(peer_jid, &room_config.users) {
            return Err(AccessDenied::NotOnRoomAllowlist);
        }

        // Per-room mention override.
        if let Some(require_mention) = room_config.require_mention {
            return if require_mention && !bot_mentioned {
                Err(AccessDenied::NotMentioned)
            } else {
                Ok(())
            };
        }
    }

    // Fall through to global mention mode.
    match config.mention_mode {
        MentionMode::Always => Ok(()),
        MentionMode::None => Err(AccessDenied::MentionModeNone),
        MentionMode::Mention => {
            if bot_mentioned {
                Ok(())
            } else {
                Err(AccessDenied::NotMentioned)
            }
        },
    }
}

/// Reason an inbound message was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDenied {
    DmsDisabled,
    NotOnAllowlist,
    GroupsDisabled,
    GroupNotOnAllowlist,
    RoomDisabled,
    NotOnRoomAllowlist,
    MentionModeNone,
    NotMentioned,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmsDisabled => write!(f, "DMs are disabled"),
            Self::NotOnAllowlist => write!(f, "user not on allowlist"),
            Self::GroupsDisabled => write!(f, "groups are disabled"),
            Self::GroupNotOnAllowlist => write!(f, "group not on allowlist"),
            Self::RoomDisabled => write!(f, "room is disabled"),
            Self::NotOnRoomAllowlist => write!(f, "user not on room allowlist"),
            Self::MentionModeNone => write!(f, "bot does not respond in groups"),
            Self::NotMentioned => write!(f, "bot was not mentioned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> XmppAccountConfig {
        XmppAccountConfig::default()
    }

    #[test]
    fn open_dm_allows_all() {
        let c = cfg();
        assert!(check_access(&c, &ChatType::Dm, "anyone@example.com", None, false).is_ok());
    }

    #[test]
    fn disabled_dm_rejects() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Disabled;
        assert_eq!(
            check_access(&c, &ChatType::Dm, "user@example.com", None, false),
            Err(AccessDenied::DmsDisabled)
        );
    }

    #[test]
    fn allowlist_dm_exact_match() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["alice@example.com".into()];
        assert!(check_access(&c, &ChatType::Dm, "alice@example.com", None, false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Dm, "bob@example.com", None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn allowlist_dm_domain_glob() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["*@trusted.org".into()];
        assert!(check_access(&c, &ChatType::Dm, "anyone@trusted.org", None, false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Dm, "user@untrusted.com", None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn group_mention_required() {
        let c = cfg(); // mention_mode=Mention by default
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                false
            ),
            Err(AccessDenied::NotMentioned)
        );
        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                true
            )
            .is_ok()
        );
    }

    #[test]
    fn group_always_mode() {
        let mut c = cfg();
        c.mention_mode = MentionMode::Always;
        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn group_disabled() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Disabled;
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                true
            ),
            Err(AccessDenied::GroupsDisabled)
        );
    }

    #[test]
    fn group_allowlist() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        c.group_allowlist = vec!["room@conf.example.com".into()];
        c.mention_mode = MentionMode::Always;
        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                false
            )
            .is_ok()
        );
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("other@conf.example.com"),
                false
            ),
            Err(AccessDenied::GroupNotOnAllowlist)
        );
    }

    #[test]
    fn per_room_disabled() {
        let mut c = cfg();
        c.mention_mode = MentionMode::Always;
        let room = crate::config::MucRoomConfig {
            enabled: false,
            ..Default::default()
        };
        c.muc_rooms.insert("room@conf.example.com".into(), room);
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                false
            ),
            Err(AccessDenied::RoomDisabled)
        );
    }

    #[test]
    fn per_room_user_allowlist() {
        let mut c = cfg();
        c.mention_mode = MentionMode::Always;
        let room = crate::config::MucRoomConfig {
            users: vec!["alice@example.com".into()],
            ..Default::default()
        };
        c.muc_rooms.insert("room@conf.example.com".into(), room);

        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "alice@example.com",
                Some("room@conf.example.com"),
                false
            )
            .is_ok()
        );
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "bob@example.com",
                Some("room@conf.example.com"),
                false
            ),
            Err(AccessDenied::NotOnRoomAllowlist)
        );
    }

    #[test]
    fn per_room_mention_override() {
        let mut c = cfg();
        c.mention_mode = MentionMode::Always; // Global: always respond

        let room = crate::config::MucRoomConfig {
            require_mention: Some(true), // Room override: require mention
            ..Default::default()
        };
        c.muc_rooms.insert("room@conf.example.com".into(), room);

        // Without mention — denied by room override.
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                false
            ),
            Err(AccessDenied::NotMentioned)
        );
        // With mention — allowed.
        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "user@example.com",
                Some("room@conf.example.com"),
                true
            )
            .is_ok()
        );
    }
}
