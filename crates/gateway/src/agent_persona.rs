//! Agent persona store for multi-agent support.
//!
//! Each agent has its own workspace directory under `data_dir()/agents/<id>/`
//! with dedicated `IDENTITY.md`, `SOUL.md`, and memory files.
//! The "main" agent always maps to the root `data_dir()` workspace.

use {
    anyhow::Result,
    serde::{Deserialize, Serialize},
    std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    },
};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// A persisted agent persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPersona {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Parameters for creating a new agent.
#[derive(Debug, Deserialize)]
pub struct CreateAgentParams {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub creature: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Parameters for updating an existing agent.
#[derive(Debug, Deserialize)]
pub struct UpdateAgentParams {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub creature: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    name: String,
    emoji: Option<String>,
    creature: Option<String>,
    vibe: Option<String>,
    description: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl From<AgentRow> for AgentPersona {
    fn from(r: AgentRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            emoji: r.emoji,
            creature: r.creature,
            vibe: r.vibe,
            description: r.description,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Validate an agent ID: lowercase alphanumeric + hyphens, 1-50 chars, not "main".
pub fn validate_agent_id(id: &str) -> Result<(), String> {
    if id == "main" {
        return Err("cannot use reserved id 'main'".into());
    }
    if id.is_empty() || id.len() > 50 {
        return Err("id must be 1-50 characters".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("id must contain only lowercase letters, digits, and hyphens".into());
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err("id must not start or end with a hyphen".into());
    }
    Ok(())
}

/// SQLite-backed agent persona store.
pub struct AgentPersonaStore {
    pool: sqlx::SqlitePool,
}

impl AgentPersonaStore {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    /// List all agents: synthesize "main" from config, then append DB rows.
    pub async fn list(&self) -> Result<Vec<AgentPersona>> {
        let main = synthesize_main_agent();
        let db_agents: Vec<AgentPersona> =
            sqlx::query_as::<_, AgentRow>("SELECT * FROM agents ORDER BY created_at ASC")
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(Into::into)
                .collect();

        let mut result = vec![main];
        result.extend(db_agents);
        Ok(result)
    }

    /// Get a single agent by ID.
    pub async fn get(&self, id: &str) -> Result<Option<AgentPersona>> {
        if id == "main" {
            return Ok(Some(synthesize_main_agent()));
        }
        let row = sqlx::query_as::<_, AgentRow>("SELECT * FROM agents WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(Into::into))
    }

    /// Create a new agent persona and its workspace directory.
    pub async fn create(&self, params: CreateAgentParams) -> Result<AgentPersona> {
        validate_agent_id(&params.id).map_err(|e| anyhow::anyhow!("{e}"))?;

        let now = now_ms();
        sqlx::query(
            r#"INSERT INTO agents (id, name, emoji, creature, vibe, description, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&params.id)
        .bind(&params.name)
        .bind(&params.emoji)
        .bind(&params.creature)
        .bind(&params.vibe)
        .bind(&params.description)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        self.ensure_workspace(&params.id)?;

        // Write initial IDENTITY.md and SOUL.md if values provided.
        let identity = moltis_config::schema::AgentIdentity {
            name: Some(params.name.clone()),
            emoji: params.emoji.clone(),
            creature: params.creature.clone(),
            vibe: params.vibe.clone(),
        };
        moltis_config::save_identity_for_agent(&params.id, &identity)?;

        Ok(AgentPersona {
            id: params.id,
            name: params.name,
            emoji: params.emoji,
            creature: params.creature,
            vibe: params.vibe,
            description: params.description,
            created_at: now,
            updated_at: now,
        })
    }

    /// Update an existing agent persona.
    pub async fn update(&self, id: &str, params: UpdateAgentParams) -> Result<AgentPersona> {
        if id == "main" {
            anyhow::bail!("cannot modify 'main' agent through this API; use identity settings");
        }

        let existing = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{id}' not found"))?;

        let name = params.name.unwrap_or(existing.name);
        let emoji = params.emoji.or(existing.emoji);
        let creature = params.creature.or(existing.creature);
        let vibe = params.vibe.or(existing.vibe);
        let description = params.description.or(existing.description);
        let now = now_ms();

        sqlx::query(
            "UPDATE agents SET name = ?, emoji = ?, creature = ?, vibe = ?, description = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&name)
        .bind(&emoji)
        .bind(&creature)
        .bind(&vibe)
        .bind(&description)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        // Update workspace IDENTITY.md.
        let identity = moltis_config::schema::AgentIdentity {
            name: Some(name.clone()),
            emoji: emoji.clone(),
            creature: creature.clone(),
            vibe: vibe.clone(),
        };
        moltis_config::save_identity_for_agent(id, &identity)?;

        Ok(AgentPersona {
            id: id.to_string(),
            name,
            emoji,
            creature,
            vibe,
            description,
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    /// Delete an agent persona. Cannot delete "main".
    pub async fn delete(&self, id: &str) -> Result<()> {
        if id == "main" {
            anyhow::bail!("cannot delete the main agent");
        }

        let result = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            anyhow::bail!("agent '{id}' not found");
        }

        // Archive the workspace directory by renaming it.
        let workspace = moltis_config::agent_data_dir(id);
        if workspace.exists() {
            let archived = workspace.with_file_name(format!("{id}.archived"));
            if let Err(e) = std::fs::rename(&workspace, &archived) {
                tracing::warn!(
                    agent_id = id,
                    error = %e,
                    "failed to archive agent workspace, removing instead"
                );
                let _ = std::fs::remove_dir_all(&workspace);
            }
        }

        Ok(())
    }

    /// Create the workspace directory for an agent.
    pub fn ensure_workspace(&self, agent_id: &str) -> Result<PathBuf> {
        let dir = moltis_config::agent_data_dir(agent_id);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// Synthesize the "main" agent persona from the global identity config.
fn synthesize_main_agent() -> AgentPersona {
    let identity = moltis_config::load_identity();
    AgentPersona {
        id: "main".to_string(),
        name: identity
            .as_ref()
            .and_then(|i| i.name.clone())
            .unwrap_or_else(|| "moltis".to_string()),
        emoji: identity.as_ref().and_then(|i| i.emoji.clone()),
        creature: identity.as_ref().and_then(|i| i.creature.clone()),
        vibe: identity.as_ref().and_then(|i| i.vibe.clone()),
        description: Some("Default agent".to_string()),
        created_at: 0,
        updated_at: 0,
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_agent_id() {
        assert!(validate_agent_id("research").is_ok());
        assert!(validate_agent_id("my-agent-1").is_ok());
        assert!(validate_agent_id("a").is_ok());

        assert!(validate_agent_id("main").is_err());
        assert!(validate_agent_id("").is_err());
        assert!(validate_agent_id("UPPER").is_err());
        assert!(validate_agent_id("has space").is_err());
        assert!(validate_agent_id("-leading").is_err());
        assert!(validate_agent_id("trailing-").is_err());
        assert!(validate_agent_id(&"a".repeat(51)).is_err());
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS agents (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                emoji       TEXT,
                creature    TEXT,
                vibe        TEXT,
                description TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            )"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_list_includes_main() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        let agents = store.list().await.unwrap();
        assert!(!agents.is_empty());
        assert_eq!(agents[0].id, "main");
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);

        let agent = store
            .create(CreateAgentParams {
                id: "research".to_string(),
                name: "Research Assistant".to_string(),
                emoji: Some("🔬".to_string()),
                creature: None,
                vibe: Some("analytical".to_string()),
                description: Some("Helps with research tasks".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(agent.id, "research");
        assert_eq!(agent.name, "Research Assistant");
        assert_eq!(agent.emoji.as_deref(), Some("🔬"));

        let fetched = store.get("research").await.unwrap().unwrap();
        assert_eq!(fetched.name, "Research Assistant");
    }

    #[tokio::test]
    async fn test_create_rejects_main() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        let result = store
            .create(CreateAgentParams {
                id: "main".to_string(),
                name: "Main".to_string(),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_rejects_invalid_id() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        let result = store
            .create(CreateAgentParams {
                id: "INVALID".to_string(),
                name: "Test".to_string(),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        store
            .create(CreateAgentParams {
                id: "writer".to_string(),
                name: "Writer".to_string(),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await
            .unwrap();

        let updated = store
            .update("writer", UpdateAgentParams {
                name: Some("Creative Writer".to_string()),
                emoji: Some("✍️".to_string()),
                creature: None,
                vibe: None,
                description: None,
            })
            .await
            .unwrap();

        assert_eq!(updated.name, "Creative Writer");
        assert_eq!(updated.emoji.as_deref(), Some("✍️"));
    }

    #[tokio::test]
    async fn test_update_main_rejected() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        let result = store
            .update("main", UpdateAgentParams {
                name: Some("Changed".to_string()),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        store
            .create(CreateAgentParams {
                id: "temp".to_string(),
                name: "Temporary".to_string(),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await
            .unwrap();

        store.delete("temp").await.unwrap();
        assert!(store.get("temp").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_main_rejected() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        assert!(store.delete("main").await.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        assert!(store.delete("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn test_list_order() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);

        store
            .create(CreateAgentParams {
                id: "beta".to_string(),
                name: "Beta".to_string(),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await
            .unwrap();

        store
            .create(CreateAgentParams {
                id: "alpha".to_string(),
                name: "Alpha".to_string(),
                emoji: None,
                creature: None,
                vibe: None,
                description: None,
            })
            .await
            .unwrap();

        let agents = store.list().await.unwrap();
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].id, "main");
        assert_eq!(agents[1].id, "beta");
        assert_eq!(agents[2].id, "alpha");
    }

    #[tokio::test]
    async fn test_get_main() {
        let pool = test_pool().await;
        let store = AgentPersonaStore::new(pool);
        let main = store.get("main").await.unwrap().unwrap();
        assert_eq!(main.id, "main");
    }
}
