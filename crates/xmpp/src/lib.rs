//! XMPP channel plugin for moltis.
//!
//! Implements `ChannelPlugin` using `tokio-xmpp` for direct stanza-level control,
//! supporting 1:1 chats and MUC (XEP-0045) group conferences.

pub mod access;
pub mod client;
pub mod config;
pub mod handlers;
pub mod outbound;
pub mod plugin;
pub mod stanza;
pub mod state;
pub mod xep;

/// Re-export tokio-xmpp's minidom to avoid version conflicts.
/// All modules in this crate should use `crate::minidom` instead of
/// importing minidom directly.
pub use tokio_xmpp::minidom;

pub use {config::XmppAccountConfig, plugin::XmppPlugin};
