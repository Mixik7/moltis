//! Shared task list tool for inter-agent task coordination.
//!
//! Provides a persistent, concurrent task list that agents can use to create,
//! claim, and track shared work items. Tasks are stored as JSON files keyed
//! by a list ID, protected by an async `RwLock`.
//!
//! # Operations
//!
//! - `create`: Add a new task (returns the assigned ID)
//! - `list`: List tasks with optional status filter
//! - `get`: Get a single task by ID
//! - `update`: Update status, subject, description, or blocked_by
//! - `claim`: Atomically set owner + status to `in_progress`

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tokio::sync::RwLock,
    tracing::debug,
};

use moltis_agents::tool_registry::AgentTool;

// ── Task types ──────────────────────────────────────────────────────────────

/// Status of a task in the shared list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            other => bail!("unknown task status: {other}"),
        }
    }
}

/// A single task in the shared list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub subject: String,
    #[serde(default)]
    pub description: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub owner: Option<String>,
    /// Task IDs that this task blocks (downstream dependents).
    #[serde(default)]
    pub blocks: Vec<String>,
    /// Task IDs that must complete before this task can start.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Persistent store for a task list, backed by a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    pub next_id: u64,
    pub tasks: HashMap<String, Task>,
}

impl Default for TaskList {
    fn default() -> Self {
        Self {
            next_id: 1,
            tasks: HashMap::new(),
        }
    }
}

// ── TaskStore ───────────────────────────────────────────────────────────────

/// Thread-safe, file-backed task store.
pub struct TaskStore {
    data_dir: PathBuf,
    lists: RwLock<HashMap<String, TaskList>>,
}

impl TaskStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.join("tasks"),
            lists: RwLock::new(HashMap::new()),
        }
    }

    fn file_path(&self, list_id: &str) -> PathBuf {
        self.data_dir.join(format!("{list_id}.json"))
    }

    /// Load a list from disk, or create a new empty one.
    async fn ensure_list(&self, list_id: &str) -> Result<()> {
        let mut lists = self.lists.write().await;
        if lists.contains_key(list_id) {
            return Ok(());
        }

        let path = self.file_path(list_id);
        let list = if path.exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            serde_json::from_str(&data)?
        } else {
            TaskList::default()
        };
        lists.insert(list_id.to_string(), list);
        Ok(())
    }

    /// Persist a list to disk.
    async fn persist(&self, list_id: &str) -> Result<()> {
        let lists = self.lists.read().await;
        if let Some(list) = lists.get(list_id) {
            tokio::fs::create_dir_all(&self.data_dir).await?;
            let data = serde_json::to_string_pretty(list)?;
            tokio::fs::write(self.file_path(list_id), data).await?;
        }
        Ok(())
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub async fn create(
        &self,
        list_id: &str,
        subject: String,
        description: String,
    ) -> Result<Task> {
        self.ensure_list(list_id).await?;
        let mut lists = self.lists.write().await;
        let list = lists.get_mut(list_id).unwrap();

        let id = list.next_id.to_string();
        list.next_id += 1;
        let now = Self::now();

        let task = Task {
            id: id.clone(),
            subject,
            description,
            status: TaskStatus::Pending,
            owner: None,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            created_at: now,
            updated_at: now,
        };

        list.tasks.insert(id, task.clone());
        drop(lists);
        self.persist(list_id).await?;
        Ok(task)
    }

    pub async fn list_tasks(
        &self,
        list_id: &str,
        status_filter: Option<&TaskStatus>,
    ) -> Result<Vec<Task>> {
        self.ensure_list(list_id).await?;
        let lists = self.lists.read().await;
        let list = lists.get(list_id).unwrap();

        let mut tasks: Vec<Task> = list
            .tasks
            .values()
            .filter(|t| status_filter.is_none_or(|s| &t.status == s))
            .cloned()
            .collect();
        tasks.sort_by_key(|t| t.id.parse::<u64>().unwrap_or(0));
        Ok(tasks)
    }

    pub async fn get(&self, list_id: &str, task_id: &str) -> Result<Option<Task>> {
        self.ensure_list(list_id).await?;
        let lists = self.lists.read().await;
        let list = lists.get(list_id).unwrap();
        Ok(list.tasks.get(task_id).cloned())
    }

    pub async fn update(
        &self,
        list_id: &str,
        task_id: &str,
        status: Option<TaskStatus>,
        subject: Option<String>,
        description: Option<String>,
        owner: Option<String>,
        blocked_by: Option<Vec<String>>,
    ) -> Result<Task> {
        self.ensure_list(list_id).await?;
        let mut lists = self.lists.write().await;
        let list = lists.get_mut(list_id).unwrap();

        let task = list
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

        if let Some(s) = status {
            task.status = s;
        }
        if let Some(s) = subject {
            task.subject = s;
        }
        if let Some(d) = description {
            task.description = d;
        }
        if let Some(o) = owner {
            task.owner = Some(o);
        }
        if let Some(deps) = blocked_by {
            task.blocked_by = deps;
        }
        task.updated_at = Self::now();

        let updated = task.clone();
        drop(lists);
        self.persist(list_id).await?;
        Ok(updated)
    }

    /// Atomically claim a task: set owner and status to `InProgress`.
    pub async fn claim(&self, list_id: &str, task_id: &str, owner: &str) -> Result<Task> {
        self.ensure_list(list_id).await?;
        let mut lists = self.lists.write().await;
        let list = lists.get_mut(list_id).unwrap();

        // Read status and dependencies before mutating.
        let (status, deps) = {
            let task = list
                .tasks
                .get(task_id)
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;
            (task.status.clone(), task.blocked_by.clone())
        };

        if status != TaskStatus::Pending {
            bail!("task {task_id} cannot be claimed: current status is {status}");
        }

        // Check if blocked by incomplete tasks.
        let blocked: Vec<String> = deps
            .iter()
            .filter(|dep_id| {
                list.tasks
                    .get(dep_id.as_str())
                    .is_some_and(|d| d.status != TaskStatus::Completed)
            })
            .cloned()
            .collect();
        if !blocked.is_empty() {
            bail!(
                "task {task_id} is blocked by incomplete tasks: {}",
                blocked.join(", ")
            );
        }

        let task = list.tasks.get_mut(task_id).unwrap();
        task.owner = Some(owner.to_string());
        task.status = TaskStatus::InProgress;
        task.updated_at = Self::now();

        let claimed = task.clone();
        drop(lists);
        self.persist(list_id).await?;
        Ok(claimed)
    }
}

// ── TaskListTool ────────────────────────────────────────────────────────────

/// Agent tool wrapping `TaskStore` for shared task coordination.
pub struct TaskListTool {
    store: Arc<TaskStore>,
}

impl TaskListTool {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            store: Arc::new(TaskStore::new(data_dir)),
        }
    }
}

#[async_trait]
impl AgentTool for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }

    fn description(&self) -> &str {
        "Manage a shared task list for coordinating work between agents. \
         Supports creating tasks, listing with filters, claiming tasks, \
         updating status, and tracking dependencies (blocked_by)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "get", "update", "claim"],
                    "description": "The operation to perform"
                },
                "list_id": {
                    "type": "string",
                    "description": "Task list identifier (default: 'default')"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (required for get, update, claim)"
                },
                "subject": {
                    "type": "string",
                    "description": "Task subject (required for create, optional for update)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "Status filter (for list) or new status (for update)"
                },
                "owner": {
                    "type": "string",
                    "description": "Owner name (required for claim, optional for update)"
                },
                "blocked_by": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs this task depends on (for update)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;
        let list_id = params["list_id"].as_str().unwrap_or("default");

        debug!(action = %action, list_id = %list_id, "task_list operation");

        match action {
            "create" => {
                let subject = params["subject"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("create requires 'subject'"))?
                    .to_string();
                let description = params["description"].as_str().unwrap_or("").to_string();
                let task = self.store.create(list_id, subject, description).await?;
                Ok(serde_json::to_value(task)?)
            },
            "list" => {
                let status_filter = params["status"]
                    .as_str()
                    .map(|s| s.parse::<TaskStatus>())
                    .transpose()?;
                let tasks = self
                    .store
                    .list_tasks(list_id, status_filter.as_ref())
                    .await?;
                Ok(serde_json::json!({
                    "tasks": serde_json::to_value(&tasks)?,
                    "count": tasks.len(),
                }))
            },
            "get" => {
                let task_id = params["task_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("get requires 'task_id'"))?;
                match self.store.get(list_id, task_id).await? {
                    Some(task) => Ok(serde_json::to_value(task)?),
                    None => bail!("task not found: {task_id}"),
                }
            },
            "update" => {
                let task_id = params["task_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("update requires 'task_id'"))?;
                let status = params["status"]
                    .as_str()
                    .map(|s| s.parse::<TaskStatus>())
                    .transpose()?;
                let subject = params["subject"].as_str().map(String::from);
                let description = params["description"].as_str().map(String::from);
                let owner = params["owner"].as_str().map(String::from);
                let blocked_by = params["blocked_by"].as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
                let task = self
                    .store
                    .update(
                        list_id,
                        task_id,
                        status,
                        subject,
                        description,
                        owner,
                        blocked_by,
                    )
                    .await?;
                Ok(serde_json::to_value(task)?)
            },
            "claim" => {
                let task_id = params["task_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("claim requires 'task_id'"))?;
                let owner = params["owner"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("claim requires 'owner'"))?;
                let task = self.store.claim(list_id, task_id, owner).await?;
                Ok(serde_json::to_value(task)?)
            },
            other => bail!("unknown action: {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> (tempfile::TempDir, Arc<TaskStore>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(TaskStore::new(dir.path()));
        (dir, store)
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let (_dir, store) = test_store().await;
        let task = store
            .create("test", "Fix bug".into(), "It crashes".into())
            .await
            .unwrap();

        assert_eq!(task.id, "1");
        assert_eq!(task.subject, "Fix bug");
        assert_eq!(task.description, "It crashes");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.owner.is_none());

        let fetched = store.get("test", "1").await.unwrap().unwrap();
        assert_eq!(fetched.subject, "Fix bug");
    }

    #[tokio::test]
    async fn test_list_with_status_filter() {
        let (_dir, store) = test_store().await;
        store
            .create("test", "Task 1".into(), String::new())
            .await
            .unwrap();
        store
            .create("test", "Task 2".into(), String::new())
            .await
            .unwrap();
        store.claim("test", "1", "agent-a").await.unwrap();

        let all = store.list_tasks("test", None).await.unwrap();
        assert_eq!(all.len(), 2);

        let pending = store
            .list_tasks("test", Some(&TaskStatus::Pending))
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "2");

        let in_progress = store
            .list_tasks("test", Some(&TaskStatus::InProgress))
            .await
            .unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].id, "1");
    }

    #[tokio::test]
    async fn test_claim_atomicity() {
        let (_dir, store) = test_store().await;
        store
            .create("test", "Task".into(), String::new())
            .await
            .unwrap();

        // First claim succeeds.
        let task = store.claim("test", "1", "agent-a").await.unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.owner.as_deref(), Some("agent-a"));

        // Second claim fails (not pending).
        let err = store.claim("test", "1", "agent-b").await.unwrap_err();
        assert!(err.to_string().contains("cannot be claimed"));
    }

    #[tokio::test]
    async fn test_blocked_by_prevents_claim() {
        let (_dir, store) = test_store().await;
        store
            .create("test", "Task 1".into(), String::new())
            .await
            .unwrap();
        store
            .create("test", "Task 2".into(), String::new())
            .await
            .unwrap();

        // Set task 2 as blocked by task 1.
        store
            .update("test", "2", None, None, None, None, Some(vec!["1".into()]))
            .await
            .unwrap();

        // Claiming task 2 should fail.
        let err = store.claim("test", "2", "agent-a").await.unwrap_err();
        assert!(err.to_string().contains("blocked by"));

        // Complete task 1, then claim task 2.
        store
            .update(
                "test",
                "1",
                Some(TaskStatus::Completed),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let task = store.claim("test", "2", "agent-a").await.unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn test_update() {
        let (_dir, store) = test_store().await;
        store
            .create("test", "Original".into(), String::new())
            .await
            .unwrap();

        let updated = store
            .update(
                "test",
                "1",
                Some(TaskStatus::InProgress),
                Some("Updated".into()),
                Some("New desc".into()),
                Some("agent-a".into()),
                None,
            )
            .await
            .unwrap();

        assert_eq!(updated.subject, "Updated");
        assert_eq!(updated.description, "New desc");
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.owner.as_deref(), Some("agent-a"));
    }

    #[tokio::test]
    async fn test_persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Create with first store instance.
        {
            let store = TaskStore::new(&path);
            store
                .create("test", "Persisted".into(), "should survive".into())
                .await
                .unwrap();
        }

        // Read with fresh store instance.
        {
            let store = TaskStore::new(&path);
            let task = store.get("test", "1").await.unwrap().unwrap();
            assert_eq!(task.subject, "Persisted");
            assert_eq!(task.description, "should survive");
        }
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let (_dir, store) = test_store().await;
        let result = store.get("test", "999").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_tool_schema() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TaskListTool::new(dir.path());

        assert_eq!(tool.name(), "task_list");
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
    }

    #[tokio::test]
    async fn test_tool_create_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TaskListTool::new(dir.path());

        // Create via tool.
        let result = tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "Test task",
                "description": "A test"
            }))
            .await
            .unwrap();
        assert_eq!(result["id"], "1");
        assert_eq!(result["status"], "pending");

        // List via tool.
        let result = tool
            .execute(serde_json::json!({ "action": "list" }))
            .await
            .unwrap();
        assert_eq!(result["count"], 1);
    }

    #[tokio::test]
    async fn test_auto_incrementing_ids() {
        let (_dir, store) = test_store().await;
        let t1 = store
            .create("test", "First".into(), String::new())
            .await
            .unwrap();
        let t2 = store
            .create("test", "Second".into(), String::new())
            .await
            .unwrap();
        let t3 = store
            .create("test", "Third".into(), String::new())
            .await
            .unwrap();

        assert_eq!(t1.id, "1");
        assert_eq!(t2.id, "2");
        assert_eq!(t3.id, "3");
    }
}
