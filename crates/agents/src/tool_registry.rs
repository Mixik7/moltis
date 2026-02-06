use {
    anyhow::Result,
    async_trait::async_trait,
    std::{collections::HashMap, sync::Arc},
};

/// Agent-callable tool.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value>;
}

/// Registry of available tools for an agent run.
///
/// Tools are stored as `Arc<dyn AgentTool>` so the registry can be cheaply
/// cloned (e.g. for sub-agents that need a filtered copy of the parent's tools).
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn AgentTool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn AgentTool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::from(tool));
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<&dyn AgentTool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list_schemas(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters_schema(),
                })
            })
            .collect()
    }

    /// Clone the registry, excluding tools whose names are in `exclude`.
    pub fn clone_without(&self, exclude: &[&str]) -> ToolRegistry {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| !exclude.contains(&name.as_str()))
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        ToolRegistry { tools }
    }

    /// Clone the registry, filtering tools based on allow/deny lists.
    ///
    /// - If `allow` is non-empty, only tools in `allow` are included.
    /// - Tools in `deny` are always excluded (applied after allow).
    /// - Tools in `always_exclude` are always excluded (e.g., spawn_agent).
    pub fn clone_with_policy(
        &self,
        allow: &[String],
        deny: &[String],
        always_exclude: &[&str],
    ) -> ToolRegistry {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| {
                // Always exclude certain tools
                if always_exclude.contains(&name.as_str()) {
                    return false;
                }
                // Check deny list
                if deny.contains(name) {
                    return false;
                }
                // If allow list is empty, allow all (minus deny/excluded)
                if allow.is_empty() {
                    return true;
                }
                // If allow list is specified, tool must be in it
                allow.contains(name)
            })
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        ToolRegistry { tools }
    }

    /// Replace an existing tool with a new one (same name).
    ///
    /// Returns `true` if a tool with that name existed and was replaced,
    /// `false` if no tool with that name was registered (the new tool is
    /// still inserted).
    pub fn replace(&mut self, tool: Box<dyn AgentTool>) -> bool {
        let name = tool.name().to_string();
        let existed = self.tools.contains_key(&name);
        self.tools.insert(name, Arc::from(tool));
        existed
    }

    /// Get the list of tool names.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}
