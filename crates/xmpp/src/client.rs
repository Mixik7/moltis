//! XMPP client event loop.
//!
//! Spawns a tokio task that owns the `tokio_xmpp::Client`, reads events
//! from it, and accepts outbound stanzas via an `mpsc` channel.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use {
    anyhow::Result,
    futures::StreamExt,
    tokio::sync::mpsc,
    tokio_util::sync::CancellationToken,
    tracing::{debug, error, info, warn},
};

use tokio_xmpp::{
    Client, Event, Stanza,
    parsers::{
        jid::BareJid,
        presence::{Presence, Type as PresenceType},
    },
};

use moltis_channels::{ChannelEventSink, message_log::MessageLog};

use crate::{
    config::XmppAccountConfig,
    handlers,
    state::{AccountState, AccountStateMap},
    xep::muc,
};

/// Size of the outbound stanza channel.
const STANZA_CHANNEL_SIZE: usize = 256;

/// Start the XMPP event loop for one account.
///
/// Creates the `tokio_xmpp::Client`, registers the account state, and spawns
/// a background task that processes events until cancelled.
pub async fn start_event_loop(
    account_id: String,
    config: XmppAccountConfig,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> Result<()> {
    let jid: BareJid = config
        .jid
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid JID '{}': {e}", config.jid))?;

    let (stanza_tx, stanza_rx) = mpsc::channel(STANZA_CHANNEL_SIZE);
    let cancel = CancellationToken::new();
    let connected = Arc::new(AtomicBool::new(false));

    // Register account state before spawning the event loop.
    {
        let state = AccountState {
            account_id: account_id.clone(),
            config: config.clone(),
            cancel: cancel.clone(),
            message_log: message_log.clone(),
            event_sink: event_sink.clone(),
            stanza_tx: stanza_tx.clone(),
            connected: Arc::clone(&connected),
        };
        let mut map = accounts.write().await;
        map.insert(account_id.clone(), state);
    }

    // Spawn the event loop task.
    let accounts_clone = Arc::clone(&accounts);
    tokio::spawn(async move {
        if let Err(e) = run_event_loop(
            account_id.clone(),
            jid,
            config,
            stanza_rx,
            cancel,
            connected,
            accounts_clone,
            message_log,
            event_sink,
        )
        .await
        {
            error!(account_id, "xmpp event loop error: {e}");
        }
    });

    Ok(())
}

/// The main event loop — owns the `tokio_xmpp::Client`.
#[allow(clippy::too_many_arguments)]
async fn run_event_loop(
    account_id: String,
    jid: BareJid,
    config: XmppAccountConfig,
    mut stanza_rx: mpsc::Receiver<crate::minidom::Element>,
    cancel: CancellationToken,
    connected: Arc<AtomicBool>,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> Result<()> {
    use secrecy::ExposeSecret;

    let password = config.password.expose_secret().to_string();
    let mut client = Client::new(jid.clone(), password);

    info!(account_id, jid = %jid, "xmpp event loop started");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!(account_id, "xmpp event loop cancelled, disconnecting");
                connected.store(false, Ordering::Relaxed);
                // Send unavailable presence before disconnecting.
                let full_jid = format!("{}/{}", config.jid, config.resource);
                let unavailable = crate::stanza::build_unavailable(&full_jid, None);
                let _ = send_raw_stanza(&mut client, unavailable).await;
                let _ = client.send_end().await;
                break;
            }

            // Outbound stanza from other tasks (via XmppOutbound).
            stanza = stanza_rx.recv() => {
                match stanza {
                    Some(element) => {
                        if let Err(e) = send_raw_stanza(&mut client, element).await {
                            warn!(account_id, "failed to send stanza: {e}");
                        }
                    }
                    None => {
                        // All senders dropped — shut down.
                        info!(account_id, "stanza channel closed, shutting down");
                        break;
                    }
                }
            }

            // Inbound event from the XMPP server.
            event = client.next() => {
                match event {
                    Some(Event::Online { bound_jid, resumed }) => {
                        info!(
                            account_id,
                            %bound_jid,
                            resumed,
                            "xmpp connected"
                        );
                        connected.store(true, Ordering::Relaxed);

                        // Send initial presence.
                        let presence = Presence::new(PresenceType::None);
                        let _ = client.send_stanza(presence.into()).await;

                        // Join configured MUC rooms.
                        let full_jid_str = format!("{}/{}", config.jid, config.resource);
                        for room in &config.rooms {
                            let room_with_nick = format!("{}/{}", room, config.resource);
                            let join = muc::build_join_presence(&full_jid_str, &room_with_nick);
                            if let Err(e) = send_raw_stanza(&mut client, join).await {
                                warn!(account_id, room, "failed to join MUC room: {e}");
                            } else {
                                debug!(account_id, room, "sent MUC join presence");
                            }
                        }
                    }

                    Some(Event::Disconnected(err)) => {
                        warn!(account_id, %err, "xmpp disconnected (will auto-reconnect)");
                        connected.store(false, Ordering::Relaxed);
                    }

                    Some(Event::Stanza(stanza)) => {
                        handlers::handle_stanza(
                            &account_id,
                            &config,
                            stanza,
                            &accounts,
                            message_log.as_ref(),
                            event_sink.as_ref(),
                        )
                        .await;
                    }

                    None => {
                        // Stream ended.
                        info!(account_id, "xmpp stream ended");
                        connected.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }
    }

    // Clean up account state on exit.
    {
        let mut map = accounts.write().await;
        map.remove(&account_id);
    }
    info!(account_id, "xmpp event loop exited");

    Ok(())
}

/// Send a raw `crate::minidom::Element` as a stanza.
///
/// Converts the Element into the appropriate `xmpp_parsers` type before
/// sending via `client.send_stanza()`.
async fn send_raw_stanza(client: &mut Client, element: crate::minidom::Element) -> Result<()> {
    // Try to convert to a known stanza type for send_stanza.
    let stanza: Stanza = match element.name() {
        "message" => {
            let msg = tokio_xmpp::parsers::message::Message::try_from(element)
                .map_err(|e| anyhow::anyhow!("invalid message stanza: {e}"))?;
            msg.into()
        },
        "presence" => {
            let pres = tokio_xmpp::parsers::presence::Presence::try_from(element)
                .map_err(|e| anyhow::anyhow!("invalid presence stanza: {e}"))?;
            pres.into()
        },
        "iq" => {
            let iq = tokio_xmpp::parsers::iq::Iq::try_from(element)
                .map_err(|e| anyhow::anyhow!("invalid iq stanza: {e}"))?;
            iq.into()
        },
        other => {
            return Err(anyhow::anyhow!("unsupported stanza type: {other}"));
        },
    };

    client
        .send_stanza(stanza)
        .await
        .map_err(|e| anyhow::anyhow!("failed to send stanza: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bare_jid() {
        let jid: BareJid = "bot@example.com".parse().unwrap();
        assert_eq!(jid.to_string(), "bot@example.com");
    }

    #[test]
    fn invalid_bare_jid() {
        let result: Result<BareJid, _> = "not a valid jid!!!".parse();
        assert!(result.is_err());
    }
}
