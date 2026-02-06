//! Shell-based hook handler for use outside the plugins crate.
//!
//! This is a lightweight version of `moltis_plugins::shell_hook::ShellHookHandler`
//! that can be used by crates (like `moltis-tools`) that don't depend on plugins.
//!
//! Protocol:
//! - Exit 0, no stdout → [`HookAction::Continue`]
//! - Exit 0, stdout JSON `{"action": "modify", "data": {...}}` → [`HookAction::ModifyPayload`]
//! - Exit 1 → [`HookAction::Block`] with stderr as reason
//! - Timeout → error (non-fatal, logged by registry)

use std::{collections::HashMap, time::Duration};

use {
    anyhow::{Context, Result, bail},
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::{io::AsyncWriteExt, process::Command},
    tracing::{debug, warn},
};

use crate::hooks::{HookAction, HookEvent, HookHandler, HookPayload};

/// Response format expected from shell hooks on stdout.
#[derive(Debug, Deserialize, Serialize)]
struct ShellHookResponse {
    action: String,
    #[serde(default)]
    data: Option<Value>,
}

/// A hook handler that executes an external shell command.
pub struct ShellHookHandler {
    hook_name: String,
    command: String,
    subscribed_events: Vec<HookEvent>,
    timeout: Duration,
    env: HashMap<String, String>,
}

impl ShellHookHandler {
    pub fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        events: Vec<HookEvent>,
        timeout: Duration,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            hook_name: name.into(),
            command: command.into(),
            subscribed_events: events,
            timeout,
            env,
        }
    }
}

#[async_trait]
impl HookHandler for ShellHookHandler {
    fn name(&self) -> &str {
        &self.hook_name
    }

    fn events(&self) -> &[HookEvent] {
        &self.subscribed_events
    }

    async fn handle(&self, _event: HookEvent, payload: &HookPayload) -> Result<HookAction> {
        let payload_json =
            serde_json::to_string(payload).context("failed to serialize hook payload")?;

        debug!(
            hook = %self.hook_name,
            command = %self.command,
            payload_len = payload_json.len(),
            "spawning shell hook"
        );

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .envs(&self.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn hook command: {}", self.command))?;

        // Write payload to stdin (ignore broken pipe if child doesn't read it).
        if let Some(mut stdin) = child.stdin.take()
            && let Err(e) = stdin.write_all(payload_json.as_bytes()).await
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(e.into());
        }

        // Wait with timeout.
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .with_context(|| {
                format!(
                    "hook '{}' timed out after {:?}",
                    self.hook_name, self.timeout
                )
            })?
            .with_context(|| format!("hook '{}' failed to complete", self.hook_name))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        debug!(
            hook = %self.hook_name,
            exit_code,
            stdout_len = stdout.len(),
            stderr_len = stderr.len(),
            "shell hook completed"
        );

        if exit_code == 1 {
            let reason = match stderr.is_empty() {
                true => format!("hook '{}' blocked the action", self.hook_name),
                false => stderr.trim().to_string(),
            };
            return Ok(HookAction::Block(reason));
        }

        if exit_code != 0 {
            bail!(
                "hook '{}' exited with code {}: {}",
                self.hook_name,
                exit_code,
                stderr.trim()
            );
        }

        // Exit 0 — check for modify response on stdout.
        let stdout_trimmed = stdout.trim();
        if stdout_trimmed.is_empty() {
            return Ok(HookAction::Continue);
        }

        match serde_json::from_str::<ShellHookResponse>(stdout_trimmed) {
            Ok(resp) if resp.action == "modify" => {
                if let Some(data) = resp.data {
                    Ok(HookAction::ModifyPayload(data))
                } else {
                    warn!(hook = %self.hook_name, "modify action without data, continuing");
                    Ok(HookAction::Continue)
                }
            },
            Ok(_) => Ok(HookAction::Continue),
            Err(e) => {
                warn!(hook = %self.hook_name, error = %e, "failed to parse hook stdout as JSON, continuing");
                Ok(HookAction::Continue)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_payload() -> HookPayload {
        HookPayload::SessionStart {
            session_key: "test-123".into(),
        }
    }

    #[tokio::test]
    async fn shell_hook_continue_on_exit_zero() {
        let handler = ShellHookHandler::new(
            "test-continue",
            "exit 0",
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn shell_hook_block_on_exit_one() {
        let handler = ShellHookHandler::new(
            "test-block",
            "echo 'blocked by policy' >&2; exit 1",
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        match result {
            HookAction::Block(reason) => assert_eq!(reason, "blocked by policy"),
            _ => panic!("expected Block"),
        }
    }

    #[tokio::test]
    async fn shell_hook_timeout() {
        let handler = ShellHookHandler::new(
            "test-timeout",
            "sleep 60",
            vec![HookEvent::SessionStart],
            Duration::from_millis(100),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("timed out"),
            "should mention timeout"
        );
    }
}
