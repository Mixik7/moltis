use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tokio_util::sync::CancellationToken;

use {
    moltis_channels::{ChannelEventSink, message_log::MessageLog},
    slack_morphism::prelude::*,
};

use crate::{config::SlackAccountConfig, outbound::SlackOutbound};

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
pub struct AccountState {
    pub client: Arc<SlackClient<SlackClientHyperConnector<SlackHyperHttpsConnector>>>,
    pub bot_user_id: Option<String>,
    pub account_id: String,
    pub config: SlackAccountConfig,
    pub outbound: Arc<SlackOutbound>,
    pub cancel: CancellationToken,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    /// Pending thread timestamps keyed by channel ID.
    pub pending_threads: HashMap<String, String>,
}
