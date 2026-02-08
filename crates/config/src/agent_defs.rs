//! Markdown-based agent definitions with YAML frontmatter.
//!
//! Scans `~/.moltis/agents/` and `.moltis/agents/` for `.md` files,
//! parsing YAML frontmatter into [`AgentPreset`] fields and using the
//! body as `system_prompt_suffix`.
//!
//! # Format
//!
//! ```markdown
//! ---
//! name: code-reviewer
//! description: Reviews code for quality
//! tools: Read, Grep, Glob
//! model: sonnet
//! memory: user
//! ---
//! System prompt body here...
//! ```

use std::{collections::HashMap, path::PathBuf};

use tracing::{debug, warn};

use crate::schema::{
    AgentIdentity, AgentPreset, MemoryScope, PresetMemoryConfig, PresetToolPolicy,
};

/// Frontmatter fields parsed from the YAML block.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AgentFrontmatter {
    name: Option<String>,
    description: Option<String>,
    tools: Option<String>,
    deny_tools: Option<String>,
    model: Option<String>,
    memory: Option<String>,
    creature: Option<String>,
    vibe: Option<String>,
    delegate_only: bool,
}

/// Parse a markdown agent definition file into a preset name and config.
///
/// Returns `(preset_name, AgentPreset)` or an error if parsing fails.
pub fn parse_agent_md(content: &str) -> anyhow::Result<(String, AgentPreset)> {
    let (frontmatter_str, body) = split_frontmatter(content)?;
    let fm: AgentFrontmatter = serde_yaml::from_str(&frontmatter_str)?;

    let name = fm
        .name
        .ok_or_else(|| anyhow::anyhow!("agent definition missing required 'name' field"))?;

    let allow = fm
        .tools
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let deny = fm
        .deny_tools
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let memory = fm.memory.map(|m| {
        let scope = match m.to_lowercase().as_str() {
            "project" => MemoryScope::Project,
            "local" => MemoryScope::Local,
            _ => MemoryScope::User,
        };
        PresetMemoryConfig {
            scope,
            ..Default::default()
        }
    });

    let body_trimmed = body.trim();
    let system_prompt_suffix = if body_trimmed.is_empty() {
        None
    } else {
        Some(body_trimmed.to_string())
    };

    let preset = AgentPreset {
        identity: AgentIdentity {
            name: Some(name.clone()),
            creature: fm.creature,
            vibe: fm.vibe,
            ..Default::default()
        },
        model: fm.model,
        tools: PresetToolPolicy { allow, deny },
        system_prompt_suffix,
        memory,
        delegate_only: fm.delegate_only,
        ..Default::default()
    };

    Ok((name, preset))
}

/// Split frontmatter (between `---` delimiters) from the body.
fn split_frontmatter(content: &str) -> anyhow::Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        anyhow::bail!("agent definition must start with '---' frontmatter delimiter");
    }

    // Skip the opening `---` and find the closing one.
    let after_open = &trimmed[3..];
    let close_pos = after_open
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("missing closing '---' frontmatter delimiter"))?;

    let frontmatter = after_open[..close_pos].to_string();
    let body = after_open[close_pos + 4..].to_string(); // skip "\n---"
    Ok((frontmatter, body))
}

/// Discover agent definition files from standard directories.
///
/// Scans `~/.moltis/agents/` (user-global) then `.moltis/agents/` (project-local).
/// Project-local files override user-global ones with the same name.
pub fn discover_agent_defs() -> HashMap<String, AgentPreset> {
    let mut defs = HashMap::new();

    // User-global: ~/.moltis/agents/
    let data_dir = crate::loader::data_dir();
    let user_dir = data_dir.join("agents");
    load_defs_from_dir(&user_dir, &mut defs);

    // Project-local: .moltis/agents/
    let project_dir = PathBuf::from(".moltis").join("agents");
    load_defs_from_dir(&project_dir, &mut defs);

    defs
}

fn load_defs_from_dir(dir: &PathBuf, defs: &mut HashMap<String, AgentPreset>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // Directory doesn't exist — that's fine.
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match std::fs::read_to_string(&path) {
                Ok(content) => match parse_agent_md(&content) {
                    Ok((name, preset)) => {
                        debug!(name = %name, path = %path.display(), "loaded agent definition");
                        defs.insert(name, preset);
                    },
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to parse agent definition");
                    },
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to read agent definition");
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_frontmatter() {
        let content = r#"---
name: reviewer
description: Reviews code
tools: Read, Grep
model: sonnet
---
You are a code reviewer. Focus on correctness.
"#;

        let (name, preset) = parse_agent_md(content).unwrap();
        assert_eq!(name, "reviewer");
        assert_eq!(preset.model, Some("sonnet".into()));
        assert_eq!(preset.tools.allow, vec!["Read", "Grep"]);
        assert_eq!(
            preset.system_prompt_suffix.as_deref(),
            Some("You are a code reviewer. Focus on correctness.")
        );
        // description from frontmatter was previously mapped to soul; now it's part of system_prompt_suffix
    }

    #[test]
    fn test_parse_full_frontmatter() {
        let content = r#"---
name: scout
description: Finds information
tools: Read, Grep, Glob
deny_tools: exec
model: haiku
memory: project
creature: owl
vibe: focused and efficient
delegate_only: false
---
Search thoroughly.
"#;

        let (name, preset) = parse_agent_md(content).unwrap();
        assert_eq!(name, "scout");
        assert_eq!(preset.tools.allow, vec!["Read", "Grep", "Glob"]);
        assert_eq!(preset.tools.deny, vec!["exec"]);
        assert_eq!(preset.identity.creature.as_deref(), Some("owl"));
        assert_eq!(
            preset.identity.vibe.as_deref(),
            Some("focused and efficient")
        );
        assert!(matches!(
            preset.memory.as_ref().unwrap().scope,
            MemoryScope::Project
        ));
        assert!(!preset.delegate_only);
    }

    #[test]
    fn test_body_becomes_system_prompt_suffix() {
        let content = "---\nname: test\n---\nThis is the system prompt.";
        let (_, preset) = parse_agent_md(content).unwrap();
        assert_eq!(
            preset.system_prompt_suffix.as_deref(),
            Some("This is the system prompt.")
        );
    }

    #[test]
    fn test_empty_body() {
        let content = "---\nname: minimal\n---\n";
        let (_, preset) = parse_agent_md(content).unwrap();
        assert!(preset.system_prompt_suffix.is_none());
    }

    #[test]
    fn test_missing_delimiters_error() {
        let content = "name: test\nno delimiters here";
        let result = parse_agent_md(content);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with '---'")
        );
    }

    #[test]
    fn test_missing_closing_delimiter() {
        let content = "---\nname: test\nno closing";
        let result = parse_agent_md(content);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing closing '---'")
        );
    }

    #[test]
    fn test_missing_name_error() {
        let content = "---\ndescription: no name\n---\nbody";
        let result = parse_agent_md(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing required"));
    }

    #[test]
    fn test_discover_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        std::fs::write(
            agents_dir.join("reviewer.md"),
            "---\nname: reviewer\n---\nReview code.",
        )
        .unwrap();
        std::fs::write(
            agents_dir.join("scout.md"),
            "---\nname: scout\ntools: Read\n---\nSearch.",
        )
        .unwrap();
        // Non-md file should be ignored.
        std::fs::write(agents_dir.join("notes.txt"), "not an agent").unwrap();

        let mut defs = HashMap::new();
        load_defs_from_dir(&agents_dir, &mut defs);

        assert_eq!(defs.len(), 2);
        assert!(defs.contains_key("reviewer"));
        assert!(defs.contains_key("scout"));
    }

    #[test]
    fn test_project_overrides_user() {
        let user_dir = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        let user_agents = user_dir.path().join("agents");
        let project_agents = project_dir.path().join("agents");
        std::fs::create_dir_all(&user_agents).unwrap();
        std::fs::create_dir_all(&project_agents).unwrap();

        // User def says model is haiku.
        std::fs::write(
            user_agents.join("reviewer.md"),
            "---\nname: reviewer\nmodel: haiku\n---\nUser version.",
        )
        .unwrap();

        // Project def says model is sonnet.
        std::fs::write(
            project_agents.join("reviewer.md"),
            "---\nname: reviewer\nmodel: sonnet\n---\nProject version.",
        )
        .unwrap();

        let mut defs = HashMap::new();
        load_defs_from_dir(&user_agents, &mut defs);
        load_defs_from_dir(&project_agents, &mut defs); // project overrides user

        assert_eq!(defs["reviewer"].model.as_deref(), Some("sonnet"));
        assert_eq!(
            defs["reviewer"].system_prompt_suffix.as_deref(),
            Some("Project version.")
        );
    }

    #[test]
    fn test_memory_scope_parsing() {
        let content = "---\nname: test\nmemory: local\n---\n";
        let (_, preset) = parse_agent_md(content).unwrap();
        assert!(matches!(
            preset.memory.as_ref().unwrap().scope,
            MemoryScope::Local
        ));

        let content = "---\nname: test\nmemory: user\n---\n";
        let (_, preset) = parse_agent_md(content).unwrap();
        assert!(matches!(
            preset.memory.as_ref().unwrap().scope,
            MemoryScope::User
        ));
    }
}
