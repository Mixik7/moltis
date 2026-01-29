use crate::tool_registry::ToolRegistry;

/// Build the system prompt for an agent run, including available tools.
pub fn build_system_prompt(tools: &ToolRegistry) -> String {
    let tool_schemas = tools.list_schemas();

    let mut prompt = String::from(
        "You are a helpful assistant with access to tools for executing shell commands.\n\n",
    );

    if !tool_schemas.is_empty() {
        prompt.push_str("## Available Tools\n\n");
        for schema in &tool_schemas {
            let name = schema["name"].as_str().unwrap_or("unknown");
            let desc = schema["description"].as_str().unwrap_or("");
            prompt.push_str(&format!("- **{name}**: {desc}\n"));
        }
        prompt.push('\n');
    }

    prompt.push_str(concat!(
        "## Guidelines\n\n",
        "- Use the exec tool to run shell commands when the user asks you to perform tasks ",
        "that require system interaction (file operations, running programs, checking status, etc.).\n",
        "- Always explain what you're doing before executing commands.\n",
        "- If a command fails, analyze the error and suggest fixes.\n",
        "- For multi-step tasks, execute commands one at a time and check results before proceeding.\n",
        "- Be careful with destructive operations — confirm with the user first.\n",
    ));

    prompt
}
