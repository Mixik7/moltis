//! Session tools for inter-agent communication.
//!
//! These tools allow agents to discover, read, and send messages to other sessions,
//! enabling asynchronous agent-to-agent coordination (the "sessions_send" pattern).
//!
//! Unlike `spawn_agent` which creates ephemeral sub-agents, these tools allow
//! communication with persistent named sessions that may be handled by different
//! agents or presets.
//!
//! # Tools
//!
//! - `sessions_list`: List active sessions with metadata
//! - `sessions_history`: Read messages from another session
//! - `sessions_send`: Send a message to another session
//!
//! # Security
//!
//! Session access is controlled by the `SessionAccessPolicy` which determines
//! which sessions an agent can see and interact with. By default, agents can
//! only access sessions with the same prefix (e.g., "agent:myagent:*").

use std::sync::Arc;

use {anyhow::Result, async_trait::async_trait, tracing::info};

use {
    moltis_agents::tool_registry::AgentTool,
    moltis_sessions::{
        metadata::{SessionEntry, SqliteSessionMetadata},
        store::SessionStore,
    },
};

/// Policy controlling which sessions an agent can access.
#[derive(Debug, Clone, Default)]
pub struct SessionAccessPolicy {
    /// If set, only sessions with keys matching this prefix are visible.
    /// E.g., "agent:myagent:" restricts to that agent's sessions.
    pub key_prefix: Option<String>,

    /// Explicit list of session keys this agent can access (in addition to prefix).
    pub allowed_keys: Vec<String>,

    /// If true, agent can send messages to other sessions.
    /// If false, only list and history are allowed.
    pub can_send: bool,

    /// If true, agent can access sessions from other agents.
    /// Requires explicit configuration in agents.toml.
    pub cross_agent: bool,
}

impl SessionAccessPolicy {
    /// Check if a session key is accessible under this policy.
    pub fn can_access(&self, key: &str) -> bool {
        // Check explicit allowed keys first.
        if self.allowed_keys.iter().any(|k| k == key) {
            return true;
        }

        // Check prefix match.
        if let Some(ref prefix) = self.key_prefix {
            return key.starts_with(prefix);
        }

        // Default: allow all if no restrictions.
        true
    }
}

impl From<&moltis_config::SessionAccessPolicyConfig> for SessionAccessPolicy {
    fn from(config: &moltis_config::SessionAccessPolicyConfig) -> Self {
        Self {
            key_prefix: config.key_prefix.clone(),
            allowed_keys: config.allowed_keys.clone(),
            can_send: config.can_send,
            cross_agent: config.cross_agent,
        }
    }
}

impl From<moltis_config::SessionAccessPolicyConfig> for SessionAccessPolicy {
    fn from(config: moltis_config::SessionAccessPolicyConfig) -> Self {
        Self::from(&config)
    }
}

// ── SessionsListTool ────────────────────────────────────────────────────────

/// Tool for listing accessible sessions.
///
/// Returns session metadata including key, label, message count, and timestamps.
/// Results are filtered by the agent's `SessionAccessPolicy`.
pub struct SessionsListTool {
    metadata: Arc<SqliteSessionMetadata>,
    policy: SessionAccessPolicy,
}

impl SessionsListTool {
    pub fn new(metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self {
            metadata,
            policy: SessionAccessPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: SessionAccessPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[async_trait]
impl AgentTool for SessionsListTool {
    fn name(&self) -> &str {
        "sessions_list"
    }

    fn description(&self) -> &str {
        "List active sessions. Use this to discover other sessions you can \
         communicate with or read history from. Returns session metadata \
         including key, label, message count, and last activity time."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Optional filter string to match session keys or labels"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of sessions to return (default: 20)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let filter = params["filter"].as_str();
        let limit = params["limit"].as_u64().unwrap_or(20) as usize;

        let all_sessions: Vec<SessionEntry> = self.metadata.list().await;

        let filtered: Vec<serde_json::Value> = all_sessions
            .into_iter()
            .filter(|s| {
                // Apply access policy.
                if !self.policy.can_access(&s.key) {
                    return false;
                }

                // Apply user filter if provided.
                if let Some(f) = filter {
                    let f_lower = f.to_lowercase();
                    let key_match = s.key.to_lowercase().contains(&f_lower);
                    let label_match = s
                        .label
                        .as_ref()
                        .map(|l| l.to_lowercase().contains(&f_lower))
                        .unwrap_or(false);
                    return key_match || label_match;
                }

                true
            })
            .take(limit)
            .map(|s| {
                serde_json::json!({
                    "key": s.key,
                    "label": s.label,
                    "messageCount": s.message_count,
                    "createdAt": s.created_at,
                    "updatedAt": s.updated_at,
                    "projectId": s.project_id,
                    "model": s.model,
                })
            })
            .collect();

        let count = filtered.len();
        Ok(serde_json::json!({
            "sessions": filtered,
            "count": count,
        }))
    }
}

// ── SessionsHistoryTool ─────────────────────────────────────────────────────

/// Tool for reading messages from another session.
///
/// Allows agents to read the conversation history of other sessions,
/// useful for understanding context or reviewing prior work.
pub struct SessionsHistoryTool {
    store: Arc<SessionStore>,
    metadata: Arc<SqliteSessionMetadata>,
    policy: SessionAccessPolicy,
}

impl SessionsHistoryTool {
    pub fn new(store: Arc<SessionStore>, metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self {
            store,
            metadata,
            policy: SessionAccessPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: SessionAccessPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[async_trait]
impl AgentTool for SessionsHistoryTool {
    fn name(&self) -> &str {
        "sessions_history"
    }

    fn description(&self) -> &str {
        "Read message history from another session. Use this to understand \
         what another agent or session has been working on, or to gather \
         context for cross-session coordination."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The session key to read history from"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of messages to return (default: 20, max: 100)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Number of messages to skip from the end (for pagination)"
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let key = params["key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;

        // Check access policy.
        if !self.policy.can_access(key) {
            anyhow::bail!("access denied: session '{key}' is not accessible");
        }

        let limit = params["limit"].as_u64().unwrap_or(20).min(100) as usize;
        let offset = params["offset"].as_u64().unwrap_or(0) as usize;

        // Get session metadata first to verify it exists.
        let meta = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| anyhow::anyhow!("session not found: {key}"))?;

        // Read messages from store.
        let all_messages: Vec<serde_json::Value> = self.store.read(key).await?;
        let total = all_messages.len();

        // Apply offset and limit (from the end, most recent first).
        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset);
        let messages: Vec<serde_json::Value> = all_messages[start..end]
            .iter()
            .map(|m| {
                // Simplify message structure for agent consumption.
                serde_json::json!({
                    "role": m["role"],
                    "content": m["content"],
                    "createdAt": m.get("created_at"),
                })
            })
            .collect();

        info!(
            session = %key,
            messages = messages.len(),
            total = total,
            "read session history"
        );

        Ok(serde_json::json!({
            "key": key,
            "label": meta.label,
            "messages": messages,
            "totalMessages": total,
            "hasMore": start > 0,
        }))
    }
}

// ── SessionsSendTool ────────────────────────────────────────────────────────

/// Callback type for sending messages to sessions.
///
/// The callback takes (session_key, message_text, wait_for_reply) and returns
/// the session's response text (if wait_for_reply is true) or an empty string.
pub type SendToSessionFn = Arc<
    dyn Fn(String, String, bool) -> futures::future::BoxFuture<'static, Result<String>>
        + Send
        + Sync,
>;

/// Tool for sending messages to another session.
///
/// This enables asynchronous agent-to-agent communication. The sending agent
/// can optionally wait for a reply, enabling request-response patterns.
pub struct SessionsSendTool {
    metadata: Arc<SqliteSessionMetadata>,
    policy: SessionAccessPolicy,
    send_fn: SendToSessionFn,
}

impl SessionsSendTool {
    pub fn new(metadata: Arc<SqliteSessionMetadata>, send_fn: SendToSessionFn) -> Self {
        Self {
            metadata,
            policy: SessionAccessPolicy {
                can_send: true,
                ..Default::default()
            },
            send_fn,
        }
    }

    pub fn with_policy(mut self, policy: SessionAccessPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[async_trait]
impl AgentTool for SessionsSendTool {
    fn name(&self) -> &str {
        "sessions_send"
    }

    fn description(&self) -> &str {
        "Send a message to another session. Use this for cross-session \
         coordination, delegating work to specialized agents, or requesting \
         information from sessions with different contexts. You can optionally \
         wait for the session to reply."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The session key to send the message to"
                },
                "message": {
                    "type": "string",
                    "description": "The message text to send"
                },
                "wait_for_reply": {
                    "type": "boolean",
                    "description": "If true, wait for the session to process and return its response (default: false)"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context to include with the message (e.g., sender identity)"
                }
            },
            "required": ["key", "message"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let key = params["key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;
        let message = params["message"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;
        let wait_for_reply = params["wait_for_reply"].as_bool().unwrap_or(false);
        let context = params["context"].as_str();

        // Check access policy.
        if !self.policy.can_access(key) {
            anyhow::bail!("access denied: session '{key}' is not accessible");
        }
        if !self.policy.can_send {
            anyhow::bail!("access denied: sending messages is not allowed by policy");
        }

        // Verify session exists.
        let meta = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| anyhow::anyhow!("session not found: {key}"))?;

        // Build the message with optional context.
        let full_message = if let Some(ctx) = context {
            format!("[From: {ctx}]\n\n{message}")
        } else {
            message.to_string()
        };

        info!(
            target_session = %key,
            wait_for_reply = wait_for_reply,
            message_len = full_message.len(),
            "sending message to session"
        );

        // Send the message.
        let reply = (self.send_fn)(key.to_string(), full_message, wait_for_reply).await?;

        if wait_for_reply {
            Ok(serde_json::json!({
                "key": key,
                "label": meta.label,
                "sent": true,
                "reply": reply,
            }))
        } else {
            Ok(serde_json::json!({
                "key": key,
                "label": meta.label,
                "sent": true,
                "message": "Message queued for delivery",
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::sync::atomic::{AtomicU32, Ordering},
    };

    /// Create an in-memory SQLite pool for testing.
    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        // Create the minimal projects table required by sessions foreign key.
        sqlx::query("CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        pool
    }

    #[test]
    fn test_access_policy_prefix() {
        let policy = SessionAccessPolicy {
            key_prefix: Some("agent:myagent:".into()),
            ..Default::default()
        };

        assert!(policy.can_access("agent:myagent:main"));
        assert!(policy.can_access("agent:myagent:work"));
        assert!(!policy.can_access("agent:other:main"));
        assert!(!policy.can_access("main"));
    }

    #[test]
    fn test_access_policy_allowed_keys() {
        let policy = SessionAccessPolicy {
            key_prefix: Some("agent:myagent:".into()),
            allowed_keys: vec!["shared:global".into()],
            ..Default::default()
        };

        assert!(policy.can_access("agent:myagent:main"));
        assert!(policy.can_access("shared:global")); // Explicit allow
        assert!(!policy.can_access("agent:other:main"));
    }

    #[test]
    fn test_access_policy_default_allows_all() {
        let policy = SessionAccessPolicy::default();

        assert!(policy.can_access("anything"));
        assert!(policy.can_access("agent:any:session"));
    }

    #[tokio::test]
    async fn test_sessions_list_tool_schema() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let tool = SessionsListTool::new(metadata);

        assert_eq!(tool.name(), "sessions_list");
        assert!(tool.description().contains("List active sessions"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["filter"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }

    #[tokio::test]
    async fn test_sessions_history_tool_schema() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let temp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(temp_dir.path().to_path_buf()));

        let tool = SessionsHistoryTool::new(store, metadata);

        assert_eq!(tool.name(), "sessions_history");
        assert!(tool.description().contains("Read message history"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"key".into())
        );
    }

    #[tokio::test]
    async fn test_sessions_send_tool_schema() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        // Mock send function.
        let send_fn: SendToSessionFn =
            Arc::new(|_key, _msg, _wait| Box::pin(async { Ok("sent".to_string()) }));

        let tool = SessionsSendTool::new(metadata, send_fn);

        assert_eq!(tool.name(), "sessions_send");
        assert!(tool.description().contains("Send a message"));

        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&"key".into()));
        assert!(required.contains(&"message".into()));
    }

    #[tokio::test]
    async fn test_sessions_list_with_filter() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        // Create some sessions.
        metadata
            .upsert("agent:main:work", Some("Work session".into()))
            .await
            .unwrap();
        metadata
            .upsert("agent:main:research", Some("Research".into()))
            .await
            .unwrap();
        metadata
            .upsert("agent:other:task", Some("Other task".into()))
            .await
            .unwrap();

        let tool = SessionsListTool::new(Arc::clone(&metadata));

        // Test without filter.
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert_eq!(result["count"], 3);

        // Test with filter.
        let result = tool
            .execute(serde_json::json!({"filter": "research"}))
            .await
            .unwrap();
        assert_eq!(result["count"], 1);
        assert_eq!(result["sessions"][0]["key"], "agent:main:research");
    }

    #[tokio::test]
    async fn test_sessions_list_with_policy() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        // Create sessions for different agents.
        metadata
            .upsert("agent:alice:main", Some("Alice main".into()))
            .await
            .unwrap();
        metadata
            .upsert("agent:alice:work", Some("Alice work".into()))
            .await
            .unwrap();
        metadata
            .upsert("agent:bob:main", Some("Bob main".into()))
            .await
            .unwrap();

        let tool = SessionsListTool::new(Arc::clone(&metadata)).with_policy(SessionAccessPolicy {
            key_prefix: Some("agent:alice:".into()),
            ..Default::default()
        });

        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert_eq!(result["count"], 2);

        // Verify only alice's sessions are returned.
        let sessions = result["sessions"].as_array().unwrap();
        for s in sessions {
            assert!(s["key"].as_str().unwrap().starts_with("agent:alice:"));
        }
    }

    #[tokio::test]
    async fn test_sessions_history_reads_messages() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let temp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(temp_dir.path().to_path_buf()));

        // Create session and add messages.
        metadata
            .upsert("test:session", Some("Test".into()))
            .await
            .unwrap();
        store
            .append(
                "test:session",
                &serde_json::json!({"role": "user", "content": "Hello"}),
            )
            .await
            .unwrap();
        store
            .append(
                "test:session",
                &serde_json::json!({"role": "assistant", "content": "Hi there!"}),
            )
            .await
            .unwrap();

        let tool = SessionsHistoryTool::new(Arc::clone(&store), Arc::clone(&metadata));

        let result = tool
            .execute(serde_json::json!({"key": "test:session"}))
            .await
            .unwrap();

        assert_eq!(result["key"], "test:session");
        assert_eq!(result["totalMessages"], 2);

        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "Hi there!");
    }

    #[tokio::test]
    async fn test_sessions_history_access_denied() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let temp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(temp_dir.path().to_path_buf()));

        metadata
            .upsert("agent:other:secret", Some("Secret".into()))
            .await
            .unwrap();

        let tool = SessionsHistoryTool::new(Arc::clone(&store), Arc::clone(&metadata)).with_policy(
            SessionAccessPolicy {
                key_prefix: Some("agent:myagent:".into()),
                ..Default::default()
            },
        );

        let result = tool
            .execute(serde_json::json!({"key": "agent:other:secret"}))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("access denied"));
    }

    #[tokio::test]
    async fn test_sessions_send_basic() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        metadata
            .upsert("target:session", Some("Target".into()))
            .await
            .unwrap();

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = Arc::clone(&call_count);

        let send_fn: SendToSessionFn = Arc::new(move |key, msg, wait| {
            let cc = Arc::clone(&call_count_clone);
            Box::pin(async move {
                cc.fetch_add(1, Ordering::SeqCst);
                assert_eq!(key, "target:session");
                assert!(msg.contains("Hello target"));
                if wait {
                    Ok("Reply from target".to_string())
                } else {
                    Ok(String::new())
                }
            })
        });

        let tool = SessionsSendTool::new(Arc::clone(&metadata), send_fn);

        // Test without wait.
        let result = tool
            .execute(serde_json::json!({
                "key": "target:session",
                "message": "Hello target"
            }))
            .await
            .unwrap();

        assert_eq!(result["sent"], true);
        assert!(result["message"].as_str().is_some());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Test with wait_for_reply.
        let result = tool
            .execute(serde_json::json!({
                "key": "target:session",
                "message": "Hello target",
                "wait_for_reply": true
            }))
            .await
            .unwrap();

        assert_eq!(result["sent"], true);
        assert_eq!(result["reply"], "Reply from target");
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_sessions_send_with_context() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        metadata
            .upsert("target:session", Some("Target".into()))
            .await
            .unwrap();

        let received_msg = Arc::new(tokio::sync::Mutex::new(String::new()));
        let received_msg_clone = Arc::clone(&received_msg);

        let send_fn: SendToSessionFn = Arc::new(move |_key, msg, _wait| {
            let rm = Arc::clone(&received_msg_clone);
            Box::pin(async move {
                *rm.lock().await = msg;
                Ok(String::new())
            })
        });

        let tool = SessionsSendTool::new(Arc::clone(&metadata), send_fn);

        tool.execute(serde_json::json!({
            "key": "target:session",
            "message": "Please help",
            "context": "researcher agent"
        }))
        .await
        .unwrap();

        let msg = received_msg.lock().await;
        assert!(msg.contains("[From: researcher agent]"));
        assert!(msg.contains("Please help"));
    }

    #[tokio::test]
    async fn test_sessions_send_policy_denied() {
        let pool = test_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        metadata
            .upsert("target:session", Some("Target".into()))
            .await
            .unwrap();

        let send_fn: SendToSessionFn =
            Arc::new(|_key, _msg, _wait| Box::pin(async { Ok(String::new()) }));

        let tool = SessionsSendTool::new(Arc::clone(&metadata), send_fn).with_policy(
            SessionAccessPolicy {
                can_send: false, // Sending disabled
                ..Default::default()
            },
        );

        let result = tool
            .execute(serde_json::json!({
                "key": "target:session",
                "message": "Hello"
            }))
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("sending messages is not allowed")
        );
    }

    #[test]
    fn test_session_access_policy_from_config() {
        let config = moltis_config::SessionAccessPolicyConfig {
            key_prefix: Some("agent:scout:".into()),
            allowed_keys: vec!["shared:global".into(), "agent:coordinator:main".into()],
            can_send: false,
            cross_agent: true,
        };

        let policy: SessionAccessPolicy = config.into();

        assert_eq!(policy.key_prefix, Some("agent:scout:".into()));
        assert_eq!(policy.allowed_keys.len(), 2);
        assert!(policy.allowed_keys.contains(&"shared:global".to_string()));
        assert!(!policy.can_send);
        assert!(policy.cross_agent);

        // Test access rules.
        assert!(policy.can_access("agent:scout:session1")); // Matches prefix.
        assert!(policy.can_access("shared:global")); // In allowed_keys.
        assert!(policy.can_access("agent:coordinator:main")); // In allowed_keys.
        assert!(!policy.can_access("agent:other:session")); // No match.
    }

    #[test]
    fn test_session_access_policy_from_config_defaults() {
        let config = moltis_config::SessionAccessPolicyConfig::default();

        let policy: SessionAccessPolicy = config.into();

        assert!(policy.key_prefix.is_none());
        assert!(policy.allowed_keys.is_empty());
        assert!(policy.can_send); // Defaults to true in config.
        assert!(!policy.cross_agent);

        // Default policy allows all.
        assert!(policy.can_access("any:session"));
    }
}
