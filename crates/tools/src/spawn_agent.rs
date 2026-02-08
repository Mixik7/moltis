//! Sub-agent tool: lets the LLM delegate tasks to a child agent loop.

use std::sync::Arc;

use {anyhow::Result, async_trait::async_trait, tokio::sync::RwLock, tracing::info};

use {
    moltis_agents::{
        model::LlmProvider,
        providers::ProviderRegistry,
        runner::{RunnerEvent, run_agent_loop_with_context},
        tool_registry::{AgentTool, ToolRegistry},
    },
    moltis_config::schema::AgentsConfig,
};

use {
    crate::sessions::{
        SendToSessionFn, SessionAccessPolicy, SessionsHistoryTool, SessionsListTool,
        SessionsSendTool,
    },
    moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
};

/// Maximum nesting depth for sub-agents (prevents infinite recursion).
const MAX_SPAWN_DEPTH: u64 = 3;

/// Tools available to delegate-only (coordinator) agents.
/// These agents can only manage sessions and tasks, not do direct work.
const DELEGATE_TOOLS: &[&str] = &[
    "spawn_agent",
    "sessions_list",
    "sessions_history",
    "sessions_send",
    "task_list",
];

/// Tool parameter injected via `tool_context` to track nesting depth.
const SPAWN_DEPTH_KEY: &str = "_spawn_depth";

/// A tool that spawns a sub-agent running its own agent loop.
///
/// The sub-agent executes synchronously (blocks until done) and its result
/// is returned as the tool output. Sub-agents get a filtered copy of the
/// parent's tool registry (without the `spawn_agent` tool itself) and a
/// focused system prompt.
///
/// When a preset is specified, the sub-agent uses that preset's model,
/// tool policies, and system prompt additions.
///
/// Callback for emitting events from the sub-agent back to the parent UI.
pub type OnSpawnEvent = Arc<dyn Fn(RunnerEvent) + Send + Sync>;

/// Dependencies for building policy-aware session tools in sub-agents.
#[derive(Clone)]
pub struct SessionDeps {
    pub session_metadata: Arc<SqliteSessionMetadata>,
    pub session_store: Arc<SessionStore>,
    pub send_to_session: SendToSessionFn,
}

pub struct SpawnAgentTool {
    provider_registry: Arc<RwLock<ProviderRegistry>>,
    default_provider: Arc<dyn LlmProvider>,
    tool_registry: Arc<ToolRegistry>,
    agents_config: Arc<RwLock<AgentsConfig>>,
    on_event: Option<OnSpawnEvent>,
    session_deps: Option<SessionDeps>,
}

impl SpawnAgentTool {
    pub fn new(
        provider_registry: Arc<RwLock<ProviderRegistry>>,
        default_provider: Arc<dyn LlmProvider>,
        tool_registry: Arc<ToolRegistry>,
        agents_config: Arc<RwLock<AgentsConfig>>,
    ) -> Self {
        Self {
            provider_registry,
            default_provider,
            tool_registry,
            agents_config,
            on_event: None,
            session_deps: None,
        }
    }

    /// Set an event callback so sub-agent activity is visible to the UI.
    pub fn with_on_event(mut self, on_event: OnSpawnEvent) -> Self {
        self.on_event = Some(on_event);
        self
    }

    /// Provide session dependencies so sub-agents can get policy-aware session tools.
    pub fn with_session_deps(mut self, deps: SessionDeps) -> Self {
        self.session_deps = Some(deps);
        self
    }

    fn emit(&self, event: RunnerEvent) {
        if let Some(ref cb) = self.on_event {
            cb(event);
        }
    }

    /// Rebuild session tools with the given policy and replace them in the registry.
    fn apply_session_policy(
        sub_tools: &mut ToolRegistry,
        deps: &SessionDeps,
        policy: SessionAccessPolicy,
    ) {
        sub_tools.replace(Box::new(
            SessionsListTool::new(Arc::clone(&deps.session_metadata)).with_policy(policy.clone()),
        ));
        sub_tools.replace(Box::new(
            SessionsHistoryTool::new(
                Arc::clone(&deps.session_store),
                Arc::clone(&deps.session_metadata),
            )
            .with_policy(policy.clone()),
        ));
        sub_tools.replace(Box::new(
            SessionsSendTool::new(
                Arc::clone(&deps.session_metadata),
                Arc::clone(&deps.send_to_session),
            )
            .with_policy(policy),
        ));
    }
}

#[async_trait]
impl AgentTool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a complex, multi-step task autonomously. \
         The sub-agent runs its own agent loop with access to tools and returns \
         the result when done. Use this to delegate tasks that require multiple \
         tool calls or independent reasoning."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task to delegate to the sub-agent"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context for the sub-agent (optional)"
                },
                "model": {
                    "type": "string",
                    "description": "Model ID to use (e.g. a cheaper model). If not specified, uses preset model or parent's model."
                },
                "preset": {
                    "type": "string",
                    "description": "Agent preset name (e.g. 'researcher', 'coder', 'reviewer'). Presets define model, tool policies, and behavior."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let task = params["task"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task"))?;
        let context = params["context"].as_str().unwrap_or("");
        let explicit_model_id = params["model"].as_str();
        let preset_name = params["preset"].as_str();

        // Check nesting depth.
        let depth = params
            .get(SPAWN_DEPTH_KEY)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if depth >= MAX_SPAWN_DEPTH {
            anyhow::bail!("maximum sub-agent nesting depth ({MAX_SPAWN_DEPTH}) exceeded");
        }

        // Load preset configuration if specified.
        let agents_config = self.agents_config.read().await;
        let preset = preset_name.and_then(|name| agents_config.get_preset(name));

        // Warn if preset was requested but not found.
        if let Some(name) = preset_name
            && preset.is_none()
        {
            info!(
                preset = name,
                "preset not found, using default configuration"
            );
        }

        // Determine model: explicit > preset > default.
        let model_id_to_use = explicit_model_id
            .map(String::from)
            .or_else(|| preset.and_then(|p| p.model.clone()));

        // Resolve provider.
        let provider = if let Some(ref id) = model_id_to_use {
            let reg = self.provider_registry.read().await;
            reg.get(id)
                .ok_or_else(|| anyhow::anyhow!("unknown model: {id}"))?
        } else {
            Arc::clone(&self.default_provider)
        };

        // Capture model ID before provider is moved into the sub-agent loop.
        let model_id = provider.id().to_string();

        // Build identity info for logging/events.
        let preset_identity_name = preset
            .and_then(|p| p.identity.name.as_ref())
            .map(String::as_str);

        info!(
            task = %task,
            depth = depth,
            model = %model_id,
            preset = ?preset_name,
            identity = ?preset_identity_name,
            "spawning sub-agent"
        );

        self.emit(RunnerEvent::SubAgentStart {
            task: task.to_string(),
            model: model_id.clone(),
            depth,
        });

        // Build filtered tool registry based on preset policy.
        let is_delegate = preset.is_some_and(|p| p.delegate_only);
        let mut sub_tools = if is_delegate {
            // Delegate mode: only delegation tools, including spawn_agent.
            let delegate_allow: Vec<String> =
                DELEGATE_TOOLS.iter().map(|s| (*s).to_string()).collect();
            self.tool_registry
                .clone_with_policy(&delegate_allow, &[], &[])
        } else if let Some(p) = preset {
            self.tool_registry.clone_with_policy(
                &p.tools.allow,
                &p.tools.deny,
                &["spawn_agent"], // Always exclude spawn_agent for non-delegates
            )
        } else {
            // Default: exclude only spawn_agent to prevent recursive spawning.
            self.tool_registry.clone_without(&["spawn_agent"])
        };

        // Apply session access policy if the preset configures one.
        if let Some(p) = preset
            && let Some(ref session_config) = p.sessions
            && let Some(ref deps) = self.session_deps
        {
            let policy = SessionAccessPolicy::from(session_config);
            Self::apply_session_policy(&mut sub_tools, deps, policy);
        }

        // Build system prompt with preset customizations.
        let mut system_prompt = build_sub_agent_prompt(task, context, preset, preset_name);

        // Inject coordinator instructions for delegate-only mode.
        if is_delegate {
            system_prompt.push_str(
                "\n\nYou are a coordinator agent. You CANNOT perform tasks directly — \
                 you do not have access to tools like exec, read, or write. Instead, \
                 you MUST delegate all work by spawning sub-agents or sending messages \
                 to other sessions. Use the task_list tool to track work items and \
                 coordinate between agents.",
            );
        }

        // Build tool context with incremented depth and propagated session key.
        let mut tool_context = serde_json::json!({
            SPAWN_DEPTH_KEY: depth + 1,
        });
        if let Some(session_key) = params.get("_session_key") {
            tool_context["_session_key"] = session_key.clone();
        }

        // Build hook registry from preset configuration, if any.
        // Must happen before dropping the agents_config read lock since
        // `preset` borrows from it.
        let hook_registry = if let Some(p) = preset
            && let Some(ref hook_configs) = p.hooks
            && !hook_configs.is_empty()
        {
            let mut registry = moltis_common::hooks::HookRegistry::new();
            for hc in hook_configs {
                let events: Vec<moltis_common::hooks::HookEvent> = hc
                    .events
                    .iter()
                    .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok())
                    .collect();
                let handler = moltis_common::shell_hook::ShellHookHandler::new(
                    hc.name.clone(),
                    hc.command.clone(),
                    events,
                    std::time::Duration::from_secs(hc.timeout),
                    hc.env.clone(),
                );
                registry.register(Arc::new(handler));
            }
            Some(Arc::new(registry))
        } else {
            None
        };

        // Drop the read lock before running the agent loop.
        drop(agents_config);

        // Run the sub-agent loop.
        let result = run_agent_loop_with_context(
            provider,
            &sub_tools,
            &system_prompt,
            task,
            None,
            None, // no history
            Some(tool_context),
            hook_registry,
        )
        .await;

        // Emit SubAgentEnd regardless of success/failure.
        let (iterations, tool_calls_made) = match &result {
            Ok(r) => (r.iterations, r.tool_calls_made),
            Err(_) => (0, 0),
        };
        self.emit(RunnerEvent::SubAgentEnd {
            task: task.to_string(),
            model: model_id.clone(),
            depth,
            iterations,
            tool_calls_made,
        });

        let result = result?;

        info!(
            task = %task,
            depth = depth,
            iterations = result.iterations,
            tool_calls = result.tool_calls_made,
            preset = ?preset_name,
            "sub-agent completed"
        );

        Ok(serde_json::json!({
            "text": result.text,
            "iterations": result.iterations,
            "tool_calls_made": result.tool_calls_made,
            "model": model_id,
            "preset": preset_name,
        }))
    }
}

/// Resolve the memory directory for a preset based on its scope.
fn resolve_memory_dir(
    preset_name: &str,
    scope: &moltis_config::schema::MemoryScope,
) -> std::path::PathBuf {
    use moltis_config::schema::MemoryScope;
    match scope {
        MemoryScope::User => {
            let data_dir = moltis_config::data_dir();
            data_dir.join("agent-memory").join(preset_name)
        },
        MemoryScope::Project => std::path::PathBuf::from(".moltis")
            .join("agent-memory")
            .join(preset_name),
        MemoryScope::Local => std::path::PathBuf::from(".moltis")
            .join("agent-memory-local")
            .join(preset_name),
    }
}

/// Load the first N lines of MEMORY.md from the agent's memory directory.
/// Returns `None` if the file doesn't exist or is empty.
fn load_memory_context(
    preset_name: &str,
    config: &moltis_config::schema::PresetMemoryConfig,
) -> Option<String> {
    let dir = resolve_memory_dir(preset_name, &config.scope);
    load_memory_from_dir(&dir, config.max_lines)
}

/// Load memory content from a specific directory.
fn load_memory_from_dir(dir: &std::path::Path, max_lines: usize) -> Option<String> {
    let memory_path = dir.join("MEMORY.md");

    // Create directory if missing so agents can write to it later.
    let _ = std::fs::create_dir_all(dir);

    let content = std::fs::read_to_string(&memory_path).ok()?;
    if content.trim().is_empty() {
        return None;
    }

    let lines: Vec<&str> = content.lines().take(max_lines).collect();
    Some(lines.join("\n"))
}

/// Build the system prompt for a sub-agent, incorporating preset customizations.
fn build_sub_agent_prompt(
    task: &str,
    context: &str,
    preset: Option<&moltis_config::schema::AgentPreset>,
    preset_name: Option<&str>,
) -> String {
    let mut prompt = String::new();

    // Add preset identity if available.
    if let Some(p) = preset {
        if let Some(ref name) = p.identity.name {
            prompt.push_str(&format!("You are {name}"));
            if let Some(ref creature) = p.identity.creature {
                prompt.push_str(&format!(", a {creature}"));
            }
            prompt.push_str(". ");
        }
        if let Some(ref vibe) = p.identity.vibe {
            prompt.push_str(&format!("Your style is {vibe}. "));
        }
    }

    // Add base instruction.
    if prompt.is_empty() {
        prompt.push_str("You are a sub-agent spawned to handle a specific task. ");
    }
    prompt.push_str("Complete the task thoroughly and return a clear result.\n\n");

    // Inject persistent memory if configured.
    if let Some(p) = preset
        && let Some(ref mem_config) = p.memory
        && let Some(name) = preset_name
        && let Some(memory_content) = load_memory_context(name, mem_config)
    {
        prompt.push_str("# Agent Memory\n\n");
        prompt.push_str(&memory_content);
        prompt.push_str("\n\n");
    }

    // Add task.
    prompt.push_str(&format!("Task: {task}"));

    // Add context if provided.
    if !context.is_empty() {
        prompt.push_str(&format!("\n\nContext: {context}"));
    }

    // Add preset system prompt suffix.
    if let Some(p) = preset
        && let Some(ref suffix) = p.system_prompt_suffix
    {
        prompt.push_str("\n\n");
        prompt.push_str(suffix);
    }

    prompt
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_agents::model::{ChatMessage, CompletionResponse, StreamEvent, Usage},
        moltis_config::schema::{AgentIdentity, AgentPreset, PresetToolPolicy},
        std::{collections::HashMap, pin::Pin},
        tokio_stream::Stream,
    };

    /// Mock provider that returns a fixed response.
    struct MockProvider {
        response: String,
        model_id: String,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn id(&self) -> &str {
            &self.model_id
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[serde_json::Value],
        ) -> Result<CompletionResponse> {
            Ok(CompletionResponse {
                text: Some(self.response.clone()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            })
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::empty())
        }
    }

    fn make_empty_provider_registry() -> Arc<RwLock<ProviderRegistry>> {
        Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &Default::default(),
        )))
    }

    fn make_empty_agents_config() -> Arc<RwLock<AgentsConfig>> {
        Arc::new(RwLock::new(AgentsConfig::default()))
    }

    fn make_spawn_tool(
        provider: Arc<dyn LlmProvider>,
        tool_registry: Arc<ToolRegistry>,
    ) -> SpawnAgentTool {
        SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            tool_registry,
            make_empty_agents_config(),
        )
    }

    #[tokio::test]
    async fn test_sub_agent_runs_and_returns_result() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "Sub-agent result".into(),
            model_id: "mock-model".into(),
        });
        let tool_registry = Arc::new(ToolRegistry::new());
        let spawn_tool = make_spawn_tool(Arc::clone(&provider), tool_registry);

        let params = serde_json::json!({ "task": "do something" });
        let result = spawn_tool.execute(params).await.unwrap();

        assert_eq!(result["text"], "Sub-agent result");
        assert_eq!(result["iterations"], 1);
        assert_eq!(result["tool_calls_made"], 0);
        assert_eq!(result["model"], "mock-model");
    }

    #[tokio::test]
    async fn test_depth_limit_rejects() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "nope".into(),
            model_id: "mock".into(),
        });
        let tool_registry = Arc::new(ToolRegistry::new());
        let spawn_tool = make_spawn_tool(provider, tool_registry);

        let params = serde_json::json!({
            "task": "do something",
            "_spawn_depth": MAX_SPAWN_DEPTH,
        });
        let result = spawn_tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nesting depth"));
    }

    #[tokio::test]
    async fn test_spawn_agent_excluded_from_sub_registry() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });

        // Create a registry with spawn_agent in it.
        let mut registry = ToolRegistry::new();

        struct DummyTool;
        #[async_trait]
        impl AgentTool for DummyTool {
            fn name(&self) -> &str {
                "spawn_agent"
            }

            fn description(&self) -> &str {
                "dummy"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, _: serde_json::Value) -> Result<serde_json::Value> {
                Ok(serde_json::json!("dummy"))
            }
        }

        struct EchoTool;
        #[async_trait]
        impl AgentTool for EchoTool {
            fn name(&self) -> &str {
                "echo"
            }

            fn description(&self) -> &str {
                "echo"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, p: serde_json::Value) -> Result<serde_json::Value> {
                Ok(p)
            }
        }

        registry.register(Box::new(DummyTool));
        registry.register(Box::new(EchoTool));

        let filtered = registry.clone_without(&["spawn_agent"]);
        assert!(filtered.get("spawn_agent").is_none());
        assert!(filtered.get("echo").is_some());

        // Also verify schemas don't include spawn_agent.
        let schemas = filtered.list_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"], "echo");

        // Ensure original is unaffected.
        assert!(registry.get("spawn_agent").is_some());

        // The SpawnAgentTool itself should work with the filtered registry.
        let spawn_tool = make_spawn_tool(provider, Arc::new(registry));
        let result = spawn_tool
            .execute(serde_json::json!({ "task": "test" }))
            .await
            .unwrap();
        assert_eq!(result["text"], "ok");
    }

    #[tokio::test]
    async fn test_context_passed_to_sub_agent() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "done with context".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = make_spawn_tool(provider, Arc::new(ToolRegistry::new()));

        let params = serde_json::json!({
            "task": "analyze code",
            "context": "The code is in src/main.rs",
        });
        let result = spawn_tool.execute(params).await.unwrap();
        assert_eq!(result["text"], "done with context");
    }

    #[tokio::test]
    async fn test_missing_task_parameter() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "nope".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = make_spawn_tool(provider, Arc::new(ToolRegistry::new()));

        let result = spawn_tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("task"));
    }

    #[tokio::test]
    async fn test_preset_applies_tool_policy() {
        // Create a registry with multiple tools.
        let mut registry = ToolRegistry::new();

        struct ReadTool;
        #[async_trait]
        impl AgentTool for ReadTool {
            fn name(&self) -> &str {
                "read_file"
            }

            fn description(&self) -> &str {
                "read"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, _: serde_json::Value) -> Result<serde_json::Value> {
                Ok(serde_json::json!("read"))
            }
        }

        struct ExecTool;
        #[async_trait]
        impl AgentTool for ExecTool {
            fn name(&self) -> &str {
                "exec"
            }

            fn description(&self) -> &str {
                "exec"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, _: serde_json::Value) -> Result<serde_json::Value> {
                Ok(serde_json::json!("exec"))
            }
        }

        registry.register(Box::new(ReadTool));
        registry.register(Box::new(ExecTool));

        // Test clone_with_policy with allow list.
        let filtered = registry.clone_with_policy(&["read_file".into()], &[], &["spawn_agent"]);
        assert!(filtered.get("read_file").is_some());
        assert!(filtered.get("exec").is_none());

        // Test clone_with_policy with deny list.
        let filtered2 = registry.clone_with_policy(&[], &["exec".into()], &["spawn_agent"]);
        assert!(filtered2.get("read_file").is_some());
        assert!(filtered2.get("exec").is_none());
    }

    #[tokio::test]
    async fn test_preset_with_identity_builds_correct_prompt() {
        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("scout".into()),
                creature: Some("helpful owl".into()),
                vibe: Some("focused and efficient".into()),
                ..Default::default()
            },
            system_prompt_suffix: Some("Focus on accuracy over speed.".into()),
            ..Default::default()
        };

        let prompt =
            build_sub_agent_prompt("find bugs", "in main.rs", Some(&preset), Some("scout"));

        assert!(prompt.contains("You are scout"));
        assert!(prompt.contains("a helpful owl"));
        assert!(prompt.contains("focused and efficient"));
        assert!(prompt.contains("Task: find bugs"));
        assert!(prompt.contains("Context: in main.rs"));
        assert!(prompt.contains("Focus on accuracy over speed"));
    }

    #[tokio::test]
    async fn test_preset_returns_in_result() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "researched".into(),
            model_id: "mock".into(),
        });

        // Create agents config with a preset.
        let mut presets = HashMap::new();
        presets.insert("researcher".into(), AgentPreset {
            identity: AgentIdentity {
                name: Some("scout".into()),
                ..Default::default()
            },
            tools: PresetToolPolicy {
                allow: vec![],
                deny: vec!["exec".into()],
            },
            ..Default::default()
        });
        let agents_config = Arc::new(RwLock::new(AgentsConfig {
            presets,
            ..Default::default()
        }));

        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
            agents_config,
        );

        let params = serde_json::json!({
            "task": "find patterns",
            "preset": "researcher",
        });
        let result = spawn_tool.execute(params).await.unwrap();

        assert_eq!(result["text"], "researched");
        assert_eq!(result["preset"], "researcher");
    }

    #[test]
    fn test_resolve_memory_dir_user_scope() {
        use moltis_config::schema::MemoryScope;
        let dir = resolve_memory_dir("scout", &MemoryScope::User);
        assert!(dir.ends_with("agent-memory/scout"));
    }

    #[test]
    fn test_resolve_memory_dir_project_scope() {
        use moltis_config::schema::MemoryScope;
        let dir = resolve_memory_dir("scout", &MemoryScope::Project);
        assert_eq!(dir, std::path::PathBuf::from(".moltis/agent-memory/scout"));
    }

    #[test]
    fn test_resolve_memory_dir_local_scope() {
        use moltis_config::schema::MemoryScope;
        let dir = resolve_memory_dir("scout", &MemoryScope::Local);
        assert_eq!(
            dir,
            std::path::PathBuf::from(".moltis/agent-memory-local/scout")
        );
    }

    #[test]
    fn test_load_memory_context_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        // No MEMORY.md file created — should return None.
        let result = load_memory_from_dir(dir.path(), 200);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_memory_context_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().to_path_buf();

        let lines: Vec<String> = (1..=10).map(|i| format!("Line {i}")).collect();
        std::fs::write(memory_dir.join("MEMORY.md"), lines.join("\n")).unwrap();

        let result = load_memory_from_dir(&memory_dir, 3).unwrap();
        assert_eq!(result, "Line 1\nLine 2\nLine 3");

        // Full read returns all lines.
        let result = load_memory_from_dir(&memory_dir, 200).unwrap();
        assert!(result.contains("Line 10"));
    }

    #[test]
    fn test_memory_injected_into_prompt() {
        use moltis_config::schema::{MemoryScope, PresetMemoryConfig};

        // To test memory injection without global state, we set a unique data_dir
        // that won't collide. We use a tempdir with a unique path.
        let dir = tempfile::tempdir().unwrap();

        // Set data_dir to our temp dir and build a matching memory file.
        moltis_config::set_data_dir(dir.path().to_path_buf());

        let memory_dir = dir.path().join("agent-memory").join("test-prompt-agent");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("MEMORY.md"), "Remember: use async").unwrap();

        let preset = AgentPreset {
            memory: Some(PresetMemoryConfig {
                scope: MemoryScope::User,
                max_lines: 200,
            }),
            ..Default::default()
        };

        let prompt =
            build_sub_agent_prompt("do work", "", Some(&preset), Some("test-prompt-agent"));
        assert!(prompt.contains("# Agent Memory"));
        assert!(prompt.contains("Remember: use async"));

        moltis_config::clear_data_dir();
    }

    #[test]
    fn test_tool_registry_replace() {
        let mut registry = ToolRegistry::new();

        struct Tool1;
        #[async_trait]
        impl AgentTool for Tool1 {
            fn name(&self) -> &str {
                "my_tool"
            }

            fn description(&self) -> &str {
                "version 1"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, _: serde_json::Value) -> Result<serde_json::Value> {
                Ok(serde_json::json!("v1"))
            }
        }

        struct Tool2;
        #[async_trait]
        impl AgentTool for Tool2 {
            fn name(&self) -> &str {
                "my_tool"
            }

            fn description(&self) -> &str {
                "version 2"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, _: serde_json::Value) -> Result<serde_json::Value> {
                Ok(serde_json::json!("v2"))
            }
        }

        registry.register(Box::new(Tool1));
        assert_eq!(registry.get("my_tool").unwrap().description(), "version 1");

        let replaced = registry.replace(Box::new(Tool2));
        assert!(replaced);
        assert_eq!(registry.get("my_tool").unwrap().description(), "version 2");
    }

    #[test]
    fn test_delegate_only_default_false() {
        let preset = AgentPreset::default();
        assert!(!preset.delegate_only);
    }

    #[test]
    fn test_delegate_only_config_parse() {
        let preset: AgentPreset = serde_json::from_str(r#"{"delegate_only": true}"#).unwrap();
        assert!(preset.delegate_only);
    }

    #[test]
    fn test_delegate_mode_tool_filtering() {
        let mut registry = ToolRegistry::new();

        // Register several tools including delegate-allowed ones.
        for name in &[
            "spawn_agent",
            "sessions_list",
            "sessions_history",
            "sessions_send",
            "task_list",
            "exec",
            "read_file",
            "web_search",
        ] {
            struct NamedTool(String);
            #[async_trait]
            impl AgentTool for NamedTool {
                fn name(&self) -> &str {
                    &self.0
                }

                fn description(&self) -> &str {
                    "test"
                }

                fn parameters_schema(&self) -> serde_json::Value {
                    serde_json::json!({})
                }

                async fn execute(&self, _: serde_json::Value) -> Result<serde_json::Value> {
                    Ok(serde_json::json!("ok"))
                }
            }
            registry.register(Box::new(NamedTool((*name).to_string())));
        }

        // Apply delegate-only filtering.
        let delegate_allow: Vec<String> = DELEGATE_TOOLS.iter().map(|s| (*s).to_string()).collect();
        let filtered = registry.clone_with_policy(&delegate_allow, &[], &[]);

        // Delegate tools should be present.
        for name in DELEGATE_TOOLS {
            assert!(
                filtered.get(name).is_some(),
                "delegate tool '{name}' should be present"
            );
        }

        // Non-delegate tools should be excluded.
        assert!(filtered.get("exec").is_none(), "exec should be excluded");
        assert!(
            filtered.get("read_file").is_none(),
            "read_file should be excluded"
        );
        assert!(
            filtered.get("web_search").is_none(),
            "web_search should be excluded"
        );

        // spawn_agent should be INCLUDED (delegates can spawn).
        assert!(
            filtered.get("spawn_agent").is_some(),
            "spawn_agent should be included for delegates"
        );
    }

    #[test]
    fn test_delegate_prompt_contains_coordinator_instructions() {
        // Verify the coordinator prompt text is what we expect.
        let coordinator_text = "You are a coordinator agent. You CANNOT perform tasks directly";
        assert!(coordinator_text.contains("coordinator"));
    }
}
