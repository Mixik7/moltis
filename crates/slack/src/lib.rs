//! Slack channel plugin for moltis.
//!
//! Implements `ChannelPlugin` using the slack-morphism library to receive and send
//! messages via the Slack API, including Socket Mode and edit-in-place streaming.

pub mod config;
pub mod markdown;
pub mod outbound;
pub mod plugin;
pub mod socket;
pub mod state;

pub use {config::SlackAccountConfig, plugin::SlackPlugin};
