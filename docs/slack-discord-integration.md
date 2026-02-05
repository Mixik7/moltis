# Slack & Discord Integration Plan for Moltis

This plan outlines the implementation of Slack and Discord channel integrations for moltis, following the established patterns from the Telegram implementation and informed by OpenClaw's approach.

## Overview

Both integrations will:
- Implement the `ChannelPlugin` trait and related interfaces
- Support bidirectional messaging (inbound → LLM → outbound)
- Provide streaming responses via edit-in-place
- Follow moltis access control patterns (DM/Group policies, allowlists)
- Be implemented as separate crates (`moltis-slack`, `moltis-discord`)

## Architecture Summary

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Gateway (server.rs)                         │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐               │
│  │  Telegram   │   │    Slack    │   │   Discord   │               │
│  │   Plugin    │   │   Plugin    │   │   Plugin    │               │
│  └──────┬──────┘   └──────┬──────┘   └──────┬──────┘               │
│         │                 │                 │                       │
│         └────────────────┬┴─────────────────┘                       │
│                          │                                          │
│                ┌─────────▼─────────┐                                │
│                │  ChannelEventSink │                                │
│                │   (dispatch_to_   │                                │
│                │      chat)        │                                │
│                └─────────┬─────────┘                                │
│                          │                                          │
│                ┌─────────▼─────────┐                                │
│                │   Chat Session    │                                │
│                │      (LLM)        │                                │
│                └─────────┬─────────┘                                │
│                          │                                          │
│         ┌────────────────┼────────────────┐                         │
│         ▼                ▼                ▼                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                 │
│  │  Telegram   │  │    Slack    │  │   Discord   │                 │
│  │  Outbound   │  │  Outbound   │  │  Outbound   │                 │
│  └─────────────┘  └─────────────┘  └─────────────┘                 │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Part 1: Slack Integration

### 1.1 Rust Library Selection

**Recommended: `slack-morphism`** ([GitHub](https://github.com/abdolence/slack-morphism-rust))

- Async-first design with tokio
- Socket Mode support (no public HTTP endpoint needed)
- Events API support (for users who want webhooks)
- Block Kit support for rich message formatting
- Well-typed API models
- Active maintenance

### 1.2 Crate Structure

```
crates/slack/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Re-exports SlackPlugin
│   ├── plugin.rs        # ChannelPlugin implementation
│   ├── config.rs        # SlackAccountConfig
│   ├── state.rs         # Per-account runtime state
│   ├── socket.rs        # Socket Mode connection handler
│   ├── handlers.rs      # Event routing (messages, reactions, etc.)
│   ├── outbound.rs      # Message sending + streaming
│   └── format.rs        # Markdown → Slack Block Kit conversion
```

### 1.3 Configuration Schema

```rust
// crates/slack/src/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackAccountConfig {
    /// Bot User OAuth Token (xoxb-...)
    #[serde(serialize_with = "serialize_secret")]
    pub bot_token: Secret<String>,

    /// App-Level Token for Socket Mode (xapp-...)
    #[serde(serialize_with = "serialize_secret")]
    pub app_token: Secret<String>,

    /// Connection mode: "socket" (default) or "events_api"
    #[serde(default = "default_socket")]
    pub mode: ConnectionMode,

    /// DM policy: open, allowlist, disabled, or pairing
    #[serde(default)]
    pub dm_policy: DmPolicy,

    /// Channel policy: open, allowlist, or disabled
    #[serde(default)]
    pub channel_policy: ChannelPolicy,

    /// User ID allowlist (supports glob patterns)
    #[serde(default)]
    pub user_allowlist: Vec<String>,

    /// Channel ID allowlist (supports glob patterns)
    #[serde(default)]
    pub channel_allowlist: Vec<String>,

    /// Activation mode in channels: "mention" (default), "always", "thread_only"
    #[serde(default)]
    pub activation_mode: ActivationMode,

    /// Default model for this Slack account
    pub model: Option<String>,

    /// Stream mode: "edit_in_place" (default) or "off"
    #[serde(default)]
    pub stream_mode: StreamMode,

    /// Throttle interval for edit updates (ms)
    #[serde(default = "default_throttle")]
    pub edit_throttle_ms: u64,

    /// History limit for channel context (0 = disabled)
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,

    /// Reply in thread by default
    #[serde(default = "default_true")]
    pub thread_replies: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    #[default]
    Mention,      // Respond only when @mentioned
    Always,       // Respond to all messages in allowed channels
    ThreadOnly,   // Only respond in threads where bot is participant
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    #[default]
    Socket,       // Socket Mode (WebSocket)
    EventsApi,    // HTTP webhook-based Events API
}
```

**Example moltis.toml:**

```toml
[channels.slack."workspace1"]
bot_token = "xoxb-..."
app_token = "xapp-..."
mode = "socket"
dm_policy = "allowlist"
channel_policy = "allowlist"
user_allowlist = ["U12345678", "admin_*"]
channel_allowlist = ["C12345678"]
activation_mode = "mention"
model = "anthropic/claude-sonnet-4-20250514"
stream_mode = "edit_in_place"
edit_throttle_ms = 500
thread_replies = true
```

### 1.4 Socket Mode Implementation

```rust
// crates/slack/src/socket.rs

pub async fn start_socket_mode(
    account_id: String,
    config: SlackAccountConfig,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> Result<()> {
    let client = SlackClient::new(SlackClientHyperConnector::new()?);

    // Create Socket Mode listener
    let socket_mode = SlackClientSocketModeConfig::new(
        config.app_token.expose_secret().into(),
    );

    let listener = SlackClientSocketModeListener::new(
        &socket_mode,
        Arc::new(SlackSocketModeListenerCallbacks::new(
            // Event handler closure
            move |event, client, state| {
                handle_socket_event(event, client, state, &config, &event_sink)
            }
        )),
    );

    // Store state with cancellation token
    let cancel = CancellationToken::new();
    {
        let mut accounts = accounts.write().unwrap();
        accounts.insert(account_id.clone(), AccountState {
            config,
            client: client.clone(),
            cancel: cancel.clone(),
        });
    }

    // Run listener until cancelled
    tokio::select! {
        result = listener.listen() => {
            if let Err(e) = result {
                error!(account_id, error = %e, "socket mode listener failed");
            }
        }
        _ = cancel.cancelled() => {
            info!(account_id, "socket mode listener cancelled");
        }
    }

    Ok(())
}
```

### 1.5 Message Handling

```rust
// crates/slack/src/handlers.rs

pub async fn handle_message_event(
    event: SlackMessageEvent,
    client: &SlackClient,
    config: &SlackAccountConfig,
    event_sink: &Option<Arc<dyn ChannelEventSink>>,
    account_id: &str,
) -> Result<()> {
    // Skip bot messages to prevent loops
    if event.bot_id.is_some() {
        return Ok(());
    }

    let channel_id = event.channel.to_string();
    let user_id = event.user.map(|u| u.to_string());
    let text = event.text.unwrap_or_default();
    let thread_ts = event.thread_ts.or(Some(event.ts.clone()));

    // Determine chat type
    let is_dm = channel_id.starts_with("D");
    let is_mention = text.contains(&format!("<@{}>", bot_user_id));

    // Access control check
    let access_granted = check_access(config, &user_id, &channel_id, is_dm);

    // Log message
    if let Some(log) = message_log {
        log.log_message(/* ... */).await?;
    }

    if !access_granted {
        return Ok(());
    }

    // Check activation mode
    let should_respond = match config.activation_mode {
        ActivationMode::Mention => is_dm || is_mention,
        ActivationMode::Always => true,
        ActivationMode::ThreadOnly => event.thread_ts.is_some(),
    };

    if !should_respond {
        return Ok(());
    }

    // Strip mention from text
    let clean_text = strip_mentions(&text);

    // Build reply target
    let reply_to = ChannelReplyTarget {
        channel_type: "slack".into(),
        account_id: account_id.to_string(),
        chat_id: channel_id.clone(),
    };

    // Extended reply target with thread info
    let reply_meta = SlackReplyMeta {
        thread_ts: if config.thread_replies { thread_ts } else { None },
        channel_id,
    };

    // Fetch channel history for context (if enabled)
    let context = if config.history_limit > 0 && !is_dm {
        fetch_channel_history(client, &channel_id, config.history_limit).await?
    } else {
        None
    };

    // Dispatch to chat
    if let Some(sink) = event_sink {
        sink.dispatch_to_chat(
            &clean_text,
            reply_to,
            ChannelMessageMeta {
                channel_type: "slack".into(),
                sender_name: user_info.real_name,
                username: user_info.name,
                model: config.model.clone(),
            },
        ).await;
    }

    Ok(())
}
```

### 1.6 Outbound & Streaming

```rust
// crates/slack/src/outbound.rs

pub struct SlackOutbound {
    pub accounts: AccountStateMap,
}

#[async_trait]
impl ChannelOutbound for SlackOutbound {
    async fn send_text(&self, account_id: &str, to: &str, text: &str) -> Result<()> {
        let (client, thread_ts) = {
            let accounts = self.accounts.read().unwrap();
            let state = accounts.get(account_id)
                .ok_or_else(|| anyhow!("account not found"))?;
            (state.client.clone(), state.pending_thread_ts.get(to).cloned())
        };

        // Convert markdown to Slack Block Kit
        let blocks = markdown_to_blocks(text);

        let req = SlackApiChatPostMessageRequest::new(
            to.into(),
            SlackMessageContent::new()
                .with_text(text.into())
                .with_blocks(blocks),
        )
        .opt_thread_ts(thread_ts);

        client.chat_post_message(&req).await?;
        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        // Slack doesn't have a typing indicator API for bots
        // This is a no-op
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for SlackOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        mut stream: StreamReceiver,
    ) -> Result<()> {
        let (client, thread_ts) = /* get from accounts */;

        // Post initial placeholder message
        let initial = client.chat_post_message(
            &SlackApiChatPostMessageRequest::new(
                to.into(),
                SlackMessageContent::new().with_text("...".into()),
            ).opt_thread_ts(thread_ts),
        ).await?;

        let message_ts = initial.ts;
        let mut accumulated = String::new();
        let mut last_update = Instant::now();
        let throttle = Duration::from_millis(config.edit_throttle_ms);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) => {
                    accumulated.push_str(&chunk);

                    // Throttle updates
                    if last_update.elapsed() >= throttle {
                        let blocks = markdown_to_blocks(&accumulated);
                        client.chat_update(
                            &SlackApiChatUpdateRequest::new(
                                to.into(),
                                SlackMessageContent::new()
                                    .with_text(accumulated.clone())
                                    .with_blocks(blocks),
                                message_ts.clone(),
                            ),
                        ).await?;
                        last_update = Instant::now();
                    }
                }
                StreamEvent::Done => {
                    // Final update with complete content
                    let blocks = markdown_to_blocks(&accumulated);
                    client.chat_update(
                        &SlackApiChatUpdateRequest::new(
                            to.into(),
                            SlackMessageContent::new()
                                .with_text(accumulated.clone())
                                .with_blocks(blocks),
                            message_ts.clone(),
                        ),
                    ).await?;
                    break;
                }
                StreamEvent::Error(e) => {
                    client.chat_update(/* error message */);
                    break;
                }
            }
        }

        Ok(())
    }
}
```

### 1.7 Slash Commands

Moltis slash commands (`/new`, `/model`, `/clear`, etc.) should be intercepted before dispatch:

```rust
// In handlers.rs

fn is_moltis_command(text: &str) -> bool {
    text.starts_with("/new") ||
    text.starts_with("/model") ||
    text.starts_with("/clear") ||
    text.starts_with("/sessions") ||
    text.starts_with("/context") ||
    text.starts_with("/compact") ||
    text.starts_with("/sandbox")
}

async fn handle_command(text: &str, reply_to: ChannelReplyTarget, event_sink: &dyn ChannelEventSink) -> Result<()> {
    let command = text.trim_start_matches('/').split_whitespace().next().unwrap_or("");
    let result = event_sink.dispatch_command(command, reply_to.clone()).await?;

    // Send command result back to Slack
    // Format as code block for readability
    Ok(())
}
```

---

## Part 2: Discord Integration

### 2.1 Rust Library Selection

**Recommended: `serenity`** ([GitHub](https://github.com/serenity-rs/serenity))

- Most popular Rust Discord library (5k+ stars)
- Async with tokio
- Gateway WebSocket connection
- Full API coverage
- Rich embed support
- Active maintenance

Consider also: **`poise`** (built on serenity) for slash command framework.

### 2.2 Crate Structure

```
crates/discord/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Re-exports DiscordPlugin
│   ├── plugin.rs        # ChannelPlugin implementation
│   ├── config.rs        # DiscordAccountConfig
│   ├── state.rs         # Per-account runtime state (Client handle)
│   ├── handler.rs       # EventHandler implementation
│   ├── outbound.rs      # Message sending + streaming
│   ├── embeds.rs        # Markdown → Discord embed conversion
│   └── commands.rs      # Slash command handlers (optional)
```

### 2.3 Configuration Schema

```rust
// crates/discord/src/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordAccountConfig {
    /// Bot token
    #[serde(serialize_with = "serialize_secret")]
    pub token: Secret<String>,

    /// DM policy: open, allowlist, disabled, or pairing
    #[serde(default)]
    pub dm_policy: DmPolicy,

    /// Server policy: open, allowlist, or disabled
    #[serde(default)]
    pub guild_policy: GuildPolicy,

    /// User ID allowlist (supports glob patterns)
    #[serde(default)]
    pub user_allowlist: Vec<String>,

    /// Guild ID allowlist
    #[serde(default)]
    pub guild_allowlist: Vec<String>,

    /// Channel ID allowlist (within allowed guilds)
    #[serde(default)]
    pub channel_allowlist: Vec<String>,

    /// Role-based access: users with these roles can interact
    #[serde(default)]
    pub role_allowlist: Vec<String>,

    /// Activation mode: "mention" (default), "always", "thread_only"
    #[serde(default)]
    pub activation_mode: ActivationMode,

    /// Default model for this Discord account
    pub model: Option<String>,

    /// Stream mode: "edit_in_place" (default) or "off"
    #[serde(default)]
    pub stream_mode: StreamMode,

    /// Throttle interval for edit updates (ms)
    #[serde(default = "default_throttle")]
    pub edit_throttle_ms: u64,

    /// History limit for guild channel context
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,

    /// Use embeds for responses (richer formatting)
    #[serde(default = "default_true")]
    pub use_embeds: bool,

    /// Register native slash commands
    #[serde(default)]
    pub native_commands: NativeCommandsMode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCommandsMode {
    #[default]
    Auto,     // Register if Discord channel is active
    On,       // Always register
    Off,      // Never register
}
```

**Example moltis.toml:**

```toml
[channels.discord."bot1"]
token = "MTIz..."
dm_policy = "open"
guild_policy = "allowlist"
guild_allowlist = ["123456789012345678"]
channel_allowlist = ["*"]  # All channels in allowed guilds
role_allowlist = ["Admin", "AI-User"]
activation_mode = "mention"
model = "anthropic/claude-sonnet-4-20250514"
stream_mode = "edit_in_place"
edit_throttle_ms = 500
use_embeds = true
native_commands = "auto"
```

### 2.4 Event Handler Implementation

```rust
// crates/discord/src/handler.rs

use serenity::{
    async_trait,
    model::{channel::Message, gateway::Ready},
    prelude::*,
};

pub struct Handler {
    account_id: String,
    config: DiscordAccountConfig,
    event_sink: Arc<dyn ChannelEventSink>,
    message_log: Option<Arc<dyn MessageLog>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            account_id = %self.account_id,
            bot_name = %ready.user.name,
            "Discord bot ready"
        );

        // Optionally register slash commands
        if matches!(self.config.native_commands, NativeCommandsMode::On | NativeCommandsMode::Auto) {
            register_commands(&ctx).await;
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Skip bot messages
        if msg.author.bot {
            return;
        }

        let channel_id = msg.channel_id.to_string();
        let user_id = msg.author.id.to_string();
        let guild_id = msg.guild_id.map(|g| g.to_string());

        // Determine if DM or guild message
        let is_dm = msg.guild_id.is_none();
        let is_mention = msg.mentions_me(&ctx.cache).await.unwrap_or(false);

        // Access control
        let access_granted = self.check_access(&user_id, &guild_id, &channel_id, is_dm, &ctx, &msg).await;

        // Log message
        if let Some(log) = &self.message_log {
            log.log_message(/* ... */).await.ok();
        }

        if !access_granted {
            return;
        }

        // Check activation mode
        let should_respond = match self.config.activation_mode {
            ActivationMode::Mention => is_dm || is_mention,
            ActivationMode::Always => true,
            ActivationMode::ThreadOnly => msg.thread.is_some(),
        };

        if !should_respond {
            return;
        }

        // Strip mentions from text
        let clean_text = strip_mentions(&msg.content, &ctx.cache).await;

        // Handle moltis commands
        if is_moltis_command(&clean_text) {
            self.handle_command(&clean_text, &msg, &ctx).await;
            return;
        }

        // Build reply target
        let reply_to = ChannelReplyTarget {
            channel_type: "discord".into(),
            account_id: self.account_id.clone(),
            chat_id: channel_id.clone(),
        };

        // Store message reference for reply threading
        {
            let mut pending = PENDING_REPLIES.write().await;
            pending.insert(
                (self.account_id.clone(), channel_id.clone()),
                msg.id,
            );
        }

        // Fetch history for context
        let context = if self.config.history_limit > 0 && !is_dm {
            self.fetch_channel_history(&ctx, msg.channel_id).await
        } else {
            None
        };

        // Dispatch to chat
        self.event_sink.dispatch_to_chat(
            &clean_text,
            reply_to,
            ChannelMessageMeta {
                channel_type: "discord".into(),
                sender_name: Some(msg.author.name.clone()),
                username: Some(msg.author.tag()),
                model: self.config.model.clone(),
            },
        ).await;
    }
}
```

### 2.5 Outbound & Streaming

```rust
// crates/discord/src/outbound.rs

pub struct DiscordOutbound {
    pub accounts: AccountStateMap,
}

#[async_trait]
impl ChannelOutbound for DiscordOutbound {
    async fn send_text(&self, account_id: &str, to: &str, text: &str) -> Result<()> {
        let http = self.get_http(account_id)?;
        let channel_id: u64 = to.parse()?;
        let channel = ChannelId::new(channel_id);

        // Get pending reply reference
        let reply_to = PENDING_REPLIES.write().await
            .remove(&(account_id.to_string(), to.to_string()));

        // Split if > 2000 chars (Discord limit)
        let chunks = split_message(text, 2000);

        for (i, chunk) in chunks.iter().enumerate() {
            let mut builder = CreateMessage::new().content(chunk);

            // Reply to original message on first chunk
            if i == 0 && let Some(msg_id) = reply_to {
                builder = builder.reference_message((channel, msg_id));
            }

            channel.send_message(&http, builder).await?;
        }

        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        let http = self.get_http(account_id)?;
        let channel_id: u64 = to.parse()?;
        ChannelId::new(channel_id).broadcast_typing(&http).await?;
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for DiscordOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        mut stream: StreamReceiver,
    ) -> Result<()> {
        let http = self.get_http(account_id)?;
        let channel_id: u64 = to.parse()?;
        let channel = ChannelId::new(channel_id);

        // Post initial placeholder
        let initial_msg = channel.send_message(
            &http,
            CreateMessage::new().content("...")
        ).await?;

        let mut accumulated = String::new();
        let mut last_update = Instant::now();
        let throttle = Duration::from_millis(config.edit_throttle_ms);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) => {
                    accumulated.push_str(&chunk);

                    if last_update.elapsed() >= throttle {
                        // Truncate if needed for Discord's 2000 char limit
                        let display = truncate_with_ellipsis(&accumulated, 2000);

                        channel.edit_message(
                            &http,
                            initial_msg.id,
                            EditMessage::new().content(&display),
                        ).await?;
                        last_update = Instant::now();
                    }
                }
                StreamEvent::Done => {
                    // Final message - may need to split if > 2000 chars
                    if accumulated.len() <= 2000 {
                        channel.edit_message(
                            &http,
                            initial_msg.id,
                            EditMessage::new().content(&accumulated),
                        ).await?;
                    } else {
                        // Delete placeholder, send as multiple messages
                        initial_msg.delete(&http).await?;
                        let chunks = split_message(&accumulated, 2000);
                        for chunk in chunks {
                            channel.send_message(
                                &http,
                                CreateMessage::new().content(&chunk),
                            ).await?;
                        }
                    }
                    break;
                }
                StreamEvent::Error(e) => {
                    channel.edit_message(
                        &http,
                        initial_msg.id,
                        EditMessage::new().content(format!("Error: {}", e)),
                    ).await?;
                    break;
                }
            }
        }

        Ok(())
    }
}
```

### 2.6 Discord-Specific Features

**Embeds for Rich Formatting:**

```rust
// crates/discord/src/embeds.rs

pub fn create_response_embed(text: &str) -> CreateEmbed {
    CreateEmbed::new()
        .description(truncate_with_ellipsis(text, 4096))  // Embed description limit
        .color(0x5865F2)  // Discord blurple
}

pub fn create_error_embed(error: &str) -> CreateEmbed {
    CreateEmbed::new()
        .title("Error")
        .description(error)
        .color(0xED4245)  // Red
}
```

**Native Slash Commands (optional):**

```rust
// crates/discord/src/commands.rs

pub async fn register_commands(ctx: &Context) -> Result<()> {
    let commands = vec![
        CreateCommand::new("new")
            .description("Start a new chat session"),
        CreateCommand::new("model")
            .description("Select AI model")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "model",
                    "Model to use"
                ).required(true)
            ),
        CreateCommand::new("clear")
            .description("Clear chat history"),
        CreateCommand::new("sessions")
            .description("List and switch sessions"),
    ];

    Command::set_global_commands(&ctx.http, commands).await?;
    Ok(())
}
```

---

## Part 3: Gateway Integration

### 3.1 Configuration Extension

```rust
// crates/config/src/schema.rs

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub slack: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub discord: HashMap<String, serde_json::Value>,
}
```

### 3.2 Server Initialization

```rust
// crates/gateway/src/server.rs (additions)

// In run() function, after Telegram initialization:

// Initialize Slack plugin
let slack_plugin = SlackPlugin::new()
    .with_message_log(message_log.clone())
    .with_event_sink(event_sink.clone());

// Load Slack accounts from config
for (account_id, config) in &config.channels.slack {
    if let Err(e) = slack_plugin.start_account(account_id, config.clone()).await {
        warn!(account_id, error = %e, "failed to start slack account");
    }
}

let slack_outbound = slack_plugin.shared_outbound();

// Initialize Discord plugin
let discord_plugin = DiscordPlugin::new()
    .with_message_log(message_log.clone())
    .with_event_sink(event_sink.clone());

// Load Discord accounts from config
for (account_id, config) in &config.channels.discord {
    if let Err(e) = discord_plugin.start_account(account_id, config.clone()).await {
        warn!(account_id, error = %e, "failed to start discord account");
    }
}

let discord_outbound = discord_plugin.shared_outbound();

// Register all channel plugins
let channel_registry = ChannelRegistry::new()
    .register(telegram_plugin)
    .register(slack_plugin)
    .register(discord_plugin);
```

### 3.3 Response Routing

```rust
// crates/gateway/src/chat.rs (update deliver_channel_replies)

pub async fn deliver_channel_replies(
    reply_targets: Vec<ChannelReplyTarget>,
    response_text: &str,
    telegram_outbound: &Arc<dyn ChannelOutbound>,
    slack_outbound: &Arc<dyn ChannelOutbound>,
    discord_outbound: &Arc<dyn ChannelOutbound>,
) {
    for target in reply_targets {
        let outbound: &Arc<dyn ChannelOutbound> = match target.channel_type.as_str() {
            "telegram" => telegram_outbound,
            "slack" => slack_outbound,
            "discord" => discord_outbound,
            other => {
                warn!(channel_type = %other, "unknown channel type");
                continue;
            }
        };

        // Spawn async task for each reply (non-blocking)
        let outbound = Arc::clone(outbound);
        let target = target.clone();
        let text = response_text.to_string();

        tokio::spawn(async move {
            if let Err(e) = outbound.send_text(&target.account_id, &target.chat_id, &text).await {
                error!(
                    channel = %target.channel_type,
                    account = %target.account_id,
                    error = %e,
                    "failed to deliver channel reply"
                );
            }
        });
    }
}
```

### 3.4 Session Key Format

```rust
// Session keys for channel messages

// Slack: slack:{account_id}:{channel_id} for channels
//        slack:{account_id}:dm:{user_id} for DMs
fn slack_session_key(account_id: &str, channel_id: &str, is_dm: bool, user_id: Option<&str>) -> String {
    if is_dm {
        format!("slack:{}:dm:{}", account_id, user_id.unwrap_or(channel_id))
    } else {
        format!("slack:{}:{}", account_id, channel_id)
    }
}

// Discord: discord:{account_id}:{guild_id}:{channel_id} for guild channels
//          discord:{account_id}:dm:{user_id} for DMs
fn discord_session_key(account_id: &str, guild_id: Option<&str>, channel_id: &str, is_dm: bool, user_id: Option<&str>) -> String {
    if is_dm {
        format!("discord:{}:dm:{}", account_id, user_id.unwrap_or(channel_id))
    } else {
        format!("discord:{}:{}:{}", account_id, guild_id.unwrap_or(""), channel_id)
    }
}
```

---

## Part 4: Implementation Phases

### Phase 1: Core Infrastructure (Week 1-2)

1. **Create crate scaffolding**
   - `crates/slack/` with Cargo.toml, basic module structure
   - `crates/discord/` with Cargo.toml, basic module structure
   - Add to workspace Cargo.toml

2. **Define configuration schemas**
   - `SlackAccountConfig` with all fields
   - `DiscordAccountConfig` with all fields
   - Update `ChannelsConfig` in schema.rs

3. **Implement plugin skeletons**
   - `SlackPlugin` implementing `ChannelPlugin` trait
   - `DiscordPlugin` implementing `ChannelPlugin` trait
   - Basic start/stop account lifecycle

### Phase 2: Slack Implementation (Week 2-3)

1. **Socket Mode connection**
   - Implement connection setup with slack-morphism
   - Handle reconnection and backoff
   - Cancellation token integration

2. **Message handling**
   - Parse message events
   - Access control (DM/channel policies, allowlists)
   - Activation mode (mention, always, thread_only)
   - Command interception

3. **Outbound messaging**
   - Implement `ChannelOutbound` for sending messages
   - Markdown → Block Kit conversion
   - Thread reply support

4. **Streaming**
   - Implement `ChannelStreamOutbound`
   - Edit-in-place with throttling
   - Handle rate limits gracefully

5. **Testing**
   - Unit tests for config parsing
   - Unit tests for access control logic
   - Integration tests with mock Slack API (if feasible)

### Phase 3: Discord Implementation (Week 3-4)

1. **Gateway connection**
   - Implement connection setup with serenity
   - Handle reconnection
   - Cancellation token integration

2. **Message handling**
   - EventHandler implementation
   - Access control (DM/guild policies, allowlists, roles)
   - Activation mode
   - Command interception

3. **Outbound messaging**
   - Implement `ChannelOutbound`
   - Message splitting for 2000 char limit
   - Optional embed formatting

4. **Streaming**
   - Implement `ChannelStreamOutbound`
   - Edit-in-place with throttling
   - Handle long responses (split messages)

5. **Native slash commands (optional)**
   - Command registration
   - Interaction handling

6. **Testing**
   - Unit tests for config parsing
   - Unit tests for access control logic
   - Integration tests with mock Discord API (if feasible)

### Phase 4: Integration & Polish (Week 4-5)

1. **Gateway integration**
   - Wire plugins into server.rs
   - Update deliver_channel_replies
   - Test multi-channel scenarios

2. **UI integration**
   - Channel status display in web UI
   - Sender approval UI for new channels
   - Configuration UI (future)

3. **Documentation**
   - Update CLAUDE.md with Slack/Discord sections
   - Add configuration examples to README
   - Create setup guides

4. **End-to-end testing**
   - Test complete message flow
   - Test streaming
   - Test access control scenarios
   - Test multi-workspace/multi-guild

---

## Part 5: Dependencies

### Cargo.toml additions

```toml
# crates/slack/Cargo.toml
[dependencies]
slack-morphism = { version = "2.0", features = ["hyper", "socket-mode"] }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
anyhow = "1"
tracing = "0.1"
secrecy = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio-util = "0.7"  # For CancellationToken

# crates/discord/Cargo.toml
[dependencies]
serenity = { version = "0.12", features = ["client", "gateway", "model", "cache"] }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
anyhow = "1"
tracing = "0.1"
secrecy = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio-util = "0.7"
```

---

## Part 6: Key Considerations

### Rate Limiting

- **Slack**: 1 message per second per channel recommended
- **Discord**: 5 messages per 5 seconds per channel; edit rate limits are separate
- Implement exponential backoff for 429 responses

### Message Limits

- **Slack**: 40,000 characters per message (blocks allow more)
- **Discord**: 2,000 characters per message; 4,096 for embed description

### Threading

- **Slack**: Use `thread_ts` for reply threading; configurable
- **Discord**: Use message reference for reply threading

### Reconnection

Both libraries handle reconnection, but implement:
- Exponential backoff on connection failures
- State cleanup on disconnect
- Re-fetch message context on reconnect

### Security

- All tokens wrapped in `Secret<String>`
- Never log token values
- Validate webhook signatures (Events API)
- Implement SSRF protection for any URL handling

---

## Summary

This plan provides a complete roadmap for implementing Slack and Discord integrations in moltis:

1. **Follows established patterns** from Telegram implementation
2. **Uses proven Rust libraries** (slack-morphism, serenity)
3. **Supports full feature set**: DMs, channels, streaming, commands, access control
4. **Maintains architectural consistency** with trait-based plugin system
5. **Includes phased implementation** for manageable development

The result will be a robust multi-channel AI assistant that works seamlessly across Telegram, Slack, and Discord while sharing the same session management, tool execution, and LLM infrastructure.

---

## Sources

- [Slack Morphism for Rust](https://github.com/abdolence/slack-morphism-rust) - Async Rust client for Slack APIs
- [Serenity Discord Library](https://github.com/serenity-rs/serenity) - Rust library for Discord API
- [Slack Socket Mode Documentation](https://docs.slack.dev/apis/events-api/using-socket-mode/) - Official Slack docs
- [OpenClaw Integration Guide](https://openrouter.ai/docs/guides/guides/openclaw-integration) - OpenRouter/OpenClaw integration
