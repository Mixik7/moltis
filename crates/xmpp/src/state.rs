use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

use {tokio::sync::mpsc, tokio_util::sync::CancellationToken};

use moltis_channels::{ChannelEventSink, message_log::MessageLog};

use crate::config::XmppAccountConfig;

/// Shared account state map.
pub type AccountStateMap = Arc<tokio::sync::RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
///
/// Key difference from Telegram: `tokio_xmpp::Client` is not `Clone`.
/// We use an `mpsc::Sender<crate::minidom::Element>` to send outbound stanzas
/// to the event loop, which owns the client.
pub struct AccountState {
    pub account_id: String,
    pub config: XmppAccountConfig,
    pub cancel: CancellationToken,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    /// Channel for sending outbound stanzas to the event loop task.
    pub stanza_tx: mpsc::Sender<crate::minidom::Element>,
    /// Whether the XMPP client is currently connected.
    pub connected: Arc<AtomicBool>,
}
