use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use {serenity::all::Http, tokio_util::sync::CancellationToken};

use moltis_channels::{ChannelEventSink, message_log::MessageLog};

use crate::{config::DiscordAccountConfig, outbound::DiscordOutbound};

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
pub struct AccountState {
    pub http: Arc<Http>,
    pub bot_user_id: Option<u64>,
    pub account_id: String,
    pub config: DiscordAccountConfig,
    pub outbound: Arc<DiscordOutbound>,
    pub cancel: CancellationToken,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    /// Pending message references for replies, keyed by channel ID.
    pub pending_replies: HashMap<String, u64>,
}
