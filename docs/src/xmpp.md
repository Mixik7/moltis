# XMPP Integration

Moltis supports XMPP (Jabber) as a messaging channel, allowing you to interact with your AI assistant through any XMPP client or in Multi-User Chat (MUC) rooms. The integration uses the XMPP protocol directly via `tokio-xmpp`, supporting both 1:1 direct messages and group conversations.

## Quick Start

Add an XMPP account to your `moltis.toml`:

```toml
[channels.xmpp.my-bot]
jid = "bot@example.com"
password = "your-password"
rooms = ["team@conference.example.com"]
```

Restart Moltis and the bot will connect to your XMPP server and auto-join the configured rooms.

## Configuration

Each XMPP account is configured under `[channels.xmpp.<account_id>]`:

```toml
[channels.xmpp.my-bot]
jid = "bot@example.com"           # Bot's JID (required)
password = "your-password"         # Account password (required)
resource = "moltis"                # XMPP resource (default: "moltis")
# server = "xmpp.example.com"     # Override server hostname (optional)

# MUC rooms to auto-join on connect
rooms = [
    "team@conference.example.com",
    "dev@conference.example.com",
]
```

### Access Control

Control who can interact with the bot:

```toml
[channels.xmpp.my-bot]
# DM policy: "open" (anyone), "allowlist" (only listed), "disabled" (no DMs)
dm_policy = "open"

# Group/MUC policy: "open", "allowlist", "disabled"
group_policy = "open"

# Mention mode in MUC rooms: "mention" (respond when mentioned),
# "always" (respond to all), "none" (ignore all)
mention_mode = "mention"

# JID allowlists (supports domain wildcards like *@trusted.org)
allowlist = [
    "alice@example.com",
    "*@trusted.org",
]
group_allowlist = [
    "team@conference.example.com",
]
```

```admonish tip
The `mention` mode is the default and recommended for MUC rooms. The bot responds when its JID, resource name, or local part is mentioned in a message.
```

### Per-Room Overrides

Each MUC room can have its own configuration that overrides the global settings:

```toml
[channels.xmpp.my-bot.muc_rooms."team@conference.example.com"]
enabled = true                     # Enable/disable bot in this room
require_mention = false            # Override mention_mode for this room
system_prompt = "You are a helpful coding assistant for the dev team."
skills = ["code-review", "explain"]

# Per-room user allowlist
users = [
    "alice@example.com",
    "bob@example.com",
]
```

### Media and Message Limits

```toml
[channels.xmpp.my-bot]
text_chunk_limit = 4000            # Max chars per message (default: 4000)
media_max_mb = 20                  # Max upload size in MB (default: 20)
blocked_media_types = []           # MIME types to reject
```

### Model Override

Use a different LLM model for a specific XMPP account:

```toml
[channels.xmpp.my-bot]
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"
```

## Supported XEPs

The XMPP integration implements the following protocol extensions:

| XEP | Name | Description |
|-----|------|-------------|
| [XEP-0045](https://xmpp.org/extensions/xep-0045.html) | Multi-User Chat | Join and participate in group chat rooms |
| [XEP-0066](https://xmpp.org/extensions/xep-0066.html) | Out of Band Data | Send media URLs as attachments |
| [XEP-0085](https://xmpp.org/extensions/xep-0085.html) | Chat State Notifications | Typing indicators (composing/active/paused) |
| [XEP-0363](https://xmpp.org/extensions/xep-0363.html) | HTTP File Upload | Upload files via the server's HTTP upload service |
| [XEP-0444](https://xmpp.org/extensions/xep-0444.html) | Message Reactions | Emoji reactions on messages |

## Features

- **Auto-reconnect**: The bot automatically reconnects and re-joins rooms on connection loss
- **Self-echo filtering**: In MUC rooms, the bot ignores its own reflected messages
- **Typing indicators**: Sends composing notifications while the LLM is generating a response
- **Slash commands**: Use `/new` to start a new session, `/help` for available commands
- **Text chunking**: Long responses are split into multiple messages at natural boundaries
- **Session management**: Each conversation (DM or room) maintains its own chat session with full history

## XMPP Server Compatibility

The integration works with any standard XMPP server that supports SASL authentication and StartTLS. Tested servers include:

- [ejabberd](https://www.ejabberd.im/)
- [Prosody](https://prosody.im/)

```admonish note
For HTTP File Upload (XEP-0363) to work, your XMPP server must have an upload component configured and accessible. The bot will fall back to sending media URLs as plain text links if upload is not available.
```

## Troubleshooting

**Bot doesn't connect**: Verify the JID and password are correct. Check that the XMPP server allows the configured resource name and supports StartTLS.

**Bot doesn't respond in MUC**: By default, `mention_mode = "mention"` requires mentioning the bot. Try setting `mention_mode = "always"` for testing, or mention the bot by its JID local part (e.g., "bot: hello").

**Messages not delivered**: Check that the sender's JID is on the allowlist (if using `dm_policy = "allowlist"`) and the room is on `group_allowlist` (if using `group_policy = "allowlist"`).
