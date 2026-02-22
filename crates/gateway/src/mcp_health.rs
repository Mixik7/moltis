//! MCP health polling and auto-restart background task.
//!
//! Monitors all MCP server connections and auto-restarts any that go down.
//! Uses exponential backoff with no hard retry limit — keeps retrying every
//! MAX_BACKOFF (5 min) until the server recovers or is removed.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tracing::{info, warn};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    mcp_service::LiveMcpService,
    state::GatewayState,
};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const BASE_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// Log a prominent warning every N failed attempts.
const WARN_EVERY_N: u32 = 5;

struct RestartState {
    count: u32,
    last_attempt: Instant,
}

/// Run the health monitor loop. Checks all MCP servers periodically,
/// broadcasts status changes, and auto-restarts dead/stopped servers
/// with exponential backoff (no hard retry limit).
pub async fn run_health_monitor(state: Arc<GatewayState>, mcp: Arc<LiveMcpService>) {
    let mut prev_states: HashMap<String, String> = HashMap::new();
    let mut restart_states: HashMap<String, RestartState> = HashMap::new();

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let statuses = mcp.manager().status_all().await;

        // --- Phase 1: Detect state changes, track failures & recoveries ---
        let mut changed = false;
        for s in &statuses {
            let prev = prev_states.get(&s.name).map(String::as_str);
            if prev != Some(&s.state) {
                changed = true;
            }

            let awaiting_auth =
                s.auth_state == Some(moltis_mcp::McpAuthState::AwaitingBrowser);
            let is_down = s.state == "dead" || s.state == "stopped";

            // Start tracking a down server that we aren't already tracking.
            if is_down && s.enabled && !awaiting_auth && !restart_states.contains_key(&s.name) {
                info!(
                    server = %s.name,
                    state = %s.state,
                    "MCP server down, scheduling auto-restart"
                );
                restart_states.insert(
                    s.name.clone(),
                    RestartState {
                        count: 0,
                        // Subtract MAX_BACKOFF so the first attempt fires immediately.
                        last_attempt: Instant::now() - MAX_BACKOFF,
                    },
                );
            }

            // Clear tracking when server is back up or in OAuth flow.
            if !is_down || awaiting_auth {
                if restart_states.remove(&s.name).is_some() && s.state == "running" {
                    info!(server = %s.name, "MCP server recovered");
                }
            }

            prev_states.insert(s.name.clone(), s.state.clone());
        }

        // Remove entries for servers no longer in the registry.
        prev_states.retain(|name, _| statuses.iter().any(|s| &s.name == name));
        restart_states.retain(|name, _| statuses.iter().any(|s| &s.name == name));

        // --- Phase 2: Retry loop — runs every poll, independent of state changes ---
        let retry_keys: Vec<String> = restart_states.keys().cloned().collect();
        for name in retry_keys {
            let backoff = {
                let rs = match restart_states.get(&name) {
                    Some(rs) => rs,
                    None => continue,
                };
                std::cmp::min(
                    BASE_BACKOFF * 2u32.saturating_pow(rs.count.min(6)),
                    MAX_BACKOFF,
                )
            };

            let elapsed = restart_states
                .get(&name)
                .map(|rs| rs.last_attempt.elapsed())
                .unwrap_or_default();
            if elapsed < backoff {
                continue;
            }

            let attempt = restart_states.get(&name).map(|rs| rs.count + 1).unwrap_or(1);
            info!(server = %name, attempt, "auto-restarting MCP server");

            match mcp.manager().restart_server(&name).await {
                Ok(()) => {
                    mcp.sync_tools_if_ready().await;
                    info!(server = %name, "MCP server auto-restarted successfully");
                    restart_states.remove(&name);
                }
                Err(e) => {
                    if let Some(rs) = restart_states.get_mut(&name) {
                        rs.count += 1;
                        rs.last_attempt = Instant::now();

                        if rs.count % WARN_EVERY_N == 0 {
                            warn!(
                                server = %name,
                                error = %e,
                                attempts = rs.count,
                                "MCP auto-restart keeps failing, will keep retrying"
                            );
                        } else {
                            warn!(
                                server = %name,
                                error = %e,
                                "MCP auto-restart failed, will retry"
                            );
                        }
                    }
                }
            }
        }

        // --- Phase 3: Broadcast status changes ---
        if changed {
            let payload = serde_json::to_value(&statuses).unwrap_or_default();
            broadcast(&state, "mcp.status", payload, BroadcastOpts::default()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_growth_and_cap() {
        // Backoff: 5, 10, 20, 40, 80, 160, 300, 300, 300...
        let expected = [5, 10, 20, 40, 80, 160, 300, 300, 300];
        for (i, &want) in expected.iter().enumerate() {
            let backoff = std::cmp::min(
                BASE_BACKOFF * 2u32.saturating_pow((i as u32).min(6)),
                MAX_BACKOFF,
            );
            assert_eq!(backoff.as_secs(), want, "attempt {i}");
        }
    }

    #[test]
    fn test_max_backoff_cap() {
        let backoff = std::cmp::min(BASE_BACKOFF * 2u32.saturating_pow(10), MAX_BACKOFF);
        assert_eq!(backoff, MAX_BACKOFF);
    }
}
