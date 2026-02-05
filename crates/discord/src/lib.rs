//! Discord channel plugin for moltis.
//!
//! Implements `ChannelPlugin` using the serenity library to receive and send
//! messages via the Discord Gateway API, including edit-in-place streaming.

pub mod config;
pub mod handler;
pub mod markdown;
pub mod outbound;
pub mod plugin;
pub mod state;

pub use {config::DiscordAccountConfig, plugin::DiscordPlugin};
