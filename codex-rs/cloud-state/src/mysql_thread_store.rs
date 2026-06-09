use async_trait::async_trait;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionMeta;
use codex_protocol::protocol::SessionMetaLine;
use codex_protocol::protocol::ThreadMemoryMode;
use codex_thread_store::AppendThreadItemsParams;
use codex_thread_store::ArchiveThreadParams;
use codex_thread_store::CreateThreadParams;
use codex_thread_store::ItemPage;
use codex_thread_store::ListItemsParams;
use codex_thread_store::ListThreadsParams;
use codex_thread_store::ListTurnsParams;
use codex_thread_store::LoadThreadHistoryParams;
use codex_thread_store::ReadThreadByRolloutPathParams;
use codex_thread_store::ReadThreadParams;
use codex_thread_store::ResumeThreadParams;
use codex_thread_store::SearchThreadsParams;
use codex_thread_store::StoredThread;
use codex_thread_store::StoredThreadHistory;
use codex_thread_store::ThreadMetadataPatch;
use codex_thread_store::ThreadPage;
use codex_thread_store::ThreadSearchPage;
use codex_thread_store::ThreadStore;
use codex_thread_store::ThreadStoreError;
use codex_thread_store::ThreadStoreResult;
use codex_thread_store::TurnPage;
use codex_thread_store::UpdateThreadMetadataParams;
use serde_json::Value;
use sqlx::MySql;
use sqlx::MySqlPool;
use sqlx::Row;
use sqlx::Transaction;

use crate::CloudStateError;
use crate::MysqlCloudStateStore;
use crate::mysql_store::storage_error;

#[derive(Debug, Clone)]
pub struct MysqlCloudThreadStore {
    store: MysqlCloudStateStore,
}

impl MysqlCloudThreadStore {
    pub fn new(store: MysqlCloudStateStore) -> Self {
        Self { store }
    }

    pub fn from_database_url_lazy_with_max_connections(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, CloudStateError> {
        MysqlCloudStateStore::from_database_url_lazy_with_max_connections(
            database_url,
            max_connections,
        )
        .map(Self::new)
    }

    async fn pool(&self) -> ThreadStoreResult<MySqlPool> {
        self.store
            .create_schema()
            .await
            .map_err(thread_store_error)?;
        Ok(self.store.pool.clone())
    }
}

#[async_trait]
impl ThreadStore for MysqlCloudThreadStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, params: CreateThreadParams) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await.map_err(storage_thread_error)?;
        let now_ms = now_millis();
        let thread_id = params.thread_id.to_string();
        let create_params_json =
            serde_json::to_string(&params).map_err(serde_thread_store_error)?;
        sqlx::query(
            r#"INSERT INTO cloud_threads
(thread_id, create_params_json, metadata_patch_json, archived_at_ms, created_at_ms, updated_at_ms)
VALUES (?, ?, ?, ?, ?, ?)
ON DUPLICATE KEY UPDATE updated_at_ms = VALUES(updated_at_ms)"#,
        )
        .bind(&thread_id)
        .bind(create_params_json)
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(storage_thread_error)?;
        insert_rollout_items(
            &mut tx,
            params.thread_id,
            vec![RolloutItem::SessionMeta(session_meta_line(&params))],
        )
        .await?;
        tx.commit().await.map_err(storage_thread_error)?;
        Ok(())
    }

    async fn resume_thread(&self, params: ResumeThreadParams) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        if let Some(history) = params.history.clone() {
            let mut tx = pool.begin().await.map_err(storage_thread_error)?;
            ensure_thread_row_for_resume(&mut tx, &params).await?;
            replace_rollout_items(&mut tx, params.thread_id, history).await?;
            tx.commit().await.map_err(storage_thread_error)?;
            return Ok(());
        }
        ensure_thread_exists(&pool, params.thread_id).await
    }

    async fn append_items(&self, params: AppendThreadItemsParams) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        let mut tx = pool.begin().await.map_err(storage_thread_error)?;
        ensure_thread_exists_in_tx(&mut tx, params.thread_id).await?;
        let turn_ids = rollout_item_turn_ids(&params.items)?;
        if let Some(turn_id) = single_turn_id(turn_ids)? {
            ensure_active_thread_writer_lease_for_turn(&mut tx, params.thread_id, &turn_id).await?;
        }
        insert_rollout_items(&mut tx, params.thread_id, params.items).await?;
        sqlx::query("UPDATE cloud_threads SET updated_at_ms = ? WHERE thread_id = ?")
            .bind(now_millis())
            .bind(params.thread_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(storage_thread_error)?;
        tx.commit().await.map_err(storage_thread_error)?;
        Ok(())
    }

    async fn persist_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        ensure_thread_exists(&pool, thread_id).await
    }

    async fn flush_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        ensure_thread_exists(&pool, thread_id).await
    }

    async fn shutdown_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        ensure_thread_exists(&pool, thread_id).await
    }

    async fn discard_thread(&self, thread_id: ThreadId) -> ThreadStoreResult<()> {
        let pool = self.pool().await?;
        ensure_thread_exists(&pool, thread_id).await
    }

    async fn load_history(
        &self,
        params: LoadThreadHistoryParams,
    ) -> ThreadStoreResult<StoredThreadHistory> {
        let pool = self.pool().await?;
        let rows = sqlx::query(
            r#"SELECT item_json
FROM cloud_thread_rollout_items
WHERE thread_id = ?
ORDER BY sequence ASC"#,
        )
        .bind(params.thread_id.to_string())
        .fetch_all(&pool)
        .await
        .map_err(storage_thread_error)?;
        if rows.is_empty() {
            ensure_thread_exists(&pool, params.thread_id).await?;
        }
        let items = rows
            .into_iter()
            .map(|row| {
                let item_json: Value = row.try_get("item_json").map_err(storage_thread_error)?;
                serde_json::from_value(item_json).map_err(serde_thread_store_error)
            })
            .collect::<ThreadStoreResult<Vec<_>>>()?;
        Ok(StoredThreadHistory {
            thread_id: params.thread_id,
            items,
        })
    }

    async fn read_thread(&self, params: ReadThreadParams) -> ThreadStoreResult<StoredThread> {
        read_thread(self.pool().await?, params).await
    }

    async fn read_thread_by_rollout_path(
        &self,
        _params: ReadThreadByRolloutPathParams,
    ) -> ThreadStoreResult<StoredThread> {
        Err(ThreadStoreError::Unsupported {
            operation: "read_thread_by_rollout_path",
        })
    }

    async fn list_threads(&self, params: ListThreadsParams) -> ThreadStoreResult<ThreadPage> {
        let pool = self.pool().await?;
        let rows = sqlx::query(
            r#"SELECT thread_id
FROM cloud_threads
WHERE archived_at_ms IS NULL
ORDER BY updated_at_ms DESC
LIMIT ?"#,
        )
        .bind(u64::try_from(params.page_size).unwrap_or(u64::MAX))
        .fetch_all(&pool)
        .await
        .map_err(storage_thread_error)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let thread_id =
                thread_id_from_string(row.try_get("thread_id").map_err(storage_thread_error)?)?;
            items.push(
                read_thread(
                    pool.clone(),
                    ReadThreadParams {
                        thread_id,
                        include_archived: false,
                        include_history: false,
                    },
                )
                .await?,
            );
        }
        Ok(ThreadPage {
            items,
            next_cursor: None,
        })
    }

    async fn search_threads(
        &self,
        _params: SearchThreadsParams,
    ) -> ThreadStoreResult<ThreadSearchPage> {
        Err(ThreadStoreError::Unsupported {
            operation: "thread/search",
        })
    }

    async fn list_turns(&self, _params: ListTurnsParams) -> ThreadStoreResult<TurnPage> {
        Err(ThreadStoreError::Unsupported {
            operation: "list_turns",
        })
    }

    async fn list_items(&self, _params: ListItemsParams) -> ThreadStoreResult<ItemPage> {
        Err(ThreadStoreError::Unsupported {
            operation: "list_items",
        })
    }

    async fn update_thread_metadata(
        &self,
        params: UpdateThreadMetadataParams,
    ) -> ThreadStoreResult<StoredThread> {
        let pool = self.pool().await?;
        let patch_json = serde_json::to_string(&params.patch).map_err(serde_thread_store_error)?;
        let result = sqlx::query(
            r#"UPDATE cloud_threads
SET metadata_patch_json = ?, updated_at_ms = ?
WHERE thread_id = ?"#,
        )
        .bind(patch_json)
        .bind(now_millis())
        .bind(params.thread_id.to_string())
        .execute(&pool)
        .await
        .map_err(storage_thread_error)?;
        if result.rows_affected() == 0 {
            return Err(ThreadStoreError::ThreadNotFound {
                thread_id: params.thread_id,
            });
        }
        read_thread(
            pool,
            ReadThreadParams {
                thread_id: params.thread_id,
                include_archived: params.include_archived,
                include_history: true,
            },
        )
        .await
    }

    async fn archive_thread(&self, params: ArchiveThreadParams) -> ThreadStoreResult<()> {
        set_archived_at(self.pool().await?, params.thread_id, Some(now_millis())).await
    }

    async fn unarchive_thread(
        &self,
        params: ArchiveThreadParams,
    ) -> ThreadStoreResult<StoredThread> {
        let pool = self.pool().await?;
        set_archived_at(pool.clone(), params.thread_id, None).await?;
        read_thread(
            pool,
            ReadThreadParams {
                thread_id: params.thread_id,
                include_archived: true,
                include_history: false,
            },
        )
        .await
    }
}

async fn read_thread(pool: MySqlPool, params: ReadThreadParams) -> ThreadStoreResult<StoredThread> {
    let row = sqlx::query(
        r#"SELECT create_params_json, metadata_patch_json, archived_at_ms, created_at_ms, updated_at_ms
FROM cloud_threads
WHERE thread_id = ?"#,
    )
    .bind(params.thread_id.to_string())
    .fetch_optional(&pool)
    .await
    .map_err(storage_thread_error)?
    .ok_or(ThreadStoreError::ThreadNotFound {
        thread_id: params.thread_id,
    })?;
    let archived_at_ms: Option<i64> = row
        .try_get("archived_at_ms")
        .map_err(storage_thread_error)?;
    if archived_at_ms.is_some() && !params.include_archived {
        return Err(ThreadStoreError::ThreadNotFound {
            thread_id: params.thread_id,
        });
    }
    let create_params_json: Value = row
        .try_get("create_params_json")
        .map_err(storage_thread_error)?;
    let create_params: CreateThreadParams =
        serde_json::from_value(create_params_json).map_err(serde_thread_store_error)?;
    let metadata_patch_json: Option<Value> = row
        .try_get("metadata_patch_json")
        .map_err(storage_thread_error)?;
    let metadata_patch = metadata_patch_json
        .map(serde_json::from_value)
        .transpose()
        .map_err(serde_thread_store_error)?;
    let history = if params.include_history {
        Some(load_history_from_pool(pool.clone(), params.thread_id).await?)
    } else {
        None
    };
    Ok(stored_thread_from_row(
        params.thread_id,
        create_params,
        metadata_patch,
        row.try_get("created_at_ms").map_err(storage_thread_error)?,
        row.try_get("updated_at_ms").map_err(storage_thread_error)?,
        archived_at_ms,
        history,
    ))
}

async fn ensure_active_thread_writer_lease_for_turn(
    tx: &mut Transaction<'_, MySql>,
    thread_id: ThreadId,
    turn_id: &str,
) -> ThreadStoreResult<()> {
    let active_request: Option<String> = sqlx::query_scalar(
        r#"SELECT r.request_id
FROM cloud_requests r
JOIN cloud_thread_writer_leases l
  ON l.thread_id = r.thread_id
 AND l.request_id = r.request_id
WHERE r.thread_id = ?
  AND r.turn_id = ?
  AND r.status = 'running'
  AND l.released_at_ms IS NULL
  AND l.expires_at_ms > ?
FOR UPDATE"#,
    )
    .bind(thread_id.to_string())
    .bind(turn_id)
    .bind(now_millis())
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_thread_error)?;
    active_request
        .map(|_| ())
        .ok_or_else(|| ThreadStoreError::Conflict {
            message: format!(
                "thread {thread_id} turn {turn_id} does not hold an active writer lease"
            ),
        })
}

fn rollout_item_turn_ids(items: &[RolloutItem]) -> ThreadStoreResult<Vec<String>> {
    let mut turn_ids = Vec::new();
    for item in items {
        let item_json = serde_json::to_value(item).map_err(serde_thread_store_error)?;
        collect_turn_ids(&item_json, &mut turn_ids);
    }
    turn_ids.sort();
    turn_ids.dedup();
    Ok(turn_ids)
}

fn collect_turn_ids(value: &Value, turn_ids: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(turn_id) = map.get("turn_id").and_then(Value::as_str) {
                turn_ids.push(turn_id.to_string());
            }
            for value in map.values() {
                collect_turn_ids(value, turn_ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_turn_ids(value, turn_ids);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn single_turn_id(turn_ids: Vec<String>) -> ThreadStoreResult<Option<String>> {
    match turn_ids.as_slice() {
        [] => Ok(None),
        [turn_id] => Ok(Some(turn_id.clone())),
        _ => Err(ThreadStoreError::Conflict {
            message: format!("append contains multiple turn ids: {}", turn_ids.join(", ")),
        }),
    }
}

async fn load_history_from_pool(
    pool: MySqlPool,
    thread_id: ThreadId,
) -> ThreadStoreResult<StoredThreadHistory> {
    let rows = sqlx::query(
        r#"SELECT item_json
FROM cloud_thread_rollout_items
WHERE thread_id = ?
ORDER BY sequence ASC"#,
    )
    .bind(thread_id.to_string())
    .fetch_all(&pool)
    .await
    .map_err(storage_thread_error)?;
    let items = rows
        .into_iter()
        .map(|row| {
            let item_json: Value = row.try_get("item_json").map_err(storage_thread_error)?;
            serde_json::from_value(item_json).map_err(serde_thread_store_error)
        })
        .collect::<ThreadStoreResult<Vec<_>>>()?;
    Ok(StoredThreadHistory { thread_id, items })
}

async fn ensure_thread_row_for_resume(
    tx: &mut Transaction<'_, MySql>,
    params: &ResumeThreadParams,
) -> ThreadStoreResult<()> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM cloud_threads WHERE thread_id = ?")
        .bind(params.thread_id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage_thread_error)?;
    if exists.is_some() {
        return Ok(());
    }
    let now_ms = now_millis();
    let create_params = CreateThreadParams {
        thread_id: params.thread_id,
        forked_from_id: None,
        parent_thread_id: None,
        source: codex_protocol::protocol::SessionSource::default(),
        thread_source: None,
        base_instructions: codex_protocol::models::BaseInstructions::default(),
        dynamic_tools: Vec::new(),
        multi_agent_version: None,
        metadata: params.metadata.clone(),
    };
    let create_params_json =
        serde_json::to_string(&create_params).map_err(serde_thread_store_error)?;
    sqlx::query(
        r#"INSERT INTO cloud_threads
(thread_id, create_params_json, metadata_patch_json, archived_at_ms, created_at_ms, updated_at_ms)
VALUES (?, ?, ?, ?, ?, ?)"#,
    )
    .bind(params.thread_id.to_string())
    .bind(create_params_json)
    .bind(None::<String>)
    .bind(None::<i64>)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(storage_thread_error)?;
    Ok(())
}

async fn replace_rollout_items(
    tx: &mut Transaction<'_, MySql>,
    thread_id: ThreadId,
    items: Vec<RolloutItem>,
) -> ThreadStoreResult<()> {
    sqlx::query("DELETE FROM cloud_thread_rollout_items WHERE thread_id = ?")
        .bind(thread_id.to_string())
        .execute(&mut **tx)
        .await
        .map_err(storage_thread_error)?;
    insert_rollout_items(tx, thread_id, items).await
}

async fn insert_rollout_items(
    tx: &mut Transaction<'_, MySql>,
    thread_id: ThreadId,
    items: Vec<RolloutItem>,
) -> ThreadStoreResult<()> {
    if items.is_empty() {
        return Ok(());
    }
    let base_sequence: Option<u64> = sqlx::query_scalar(
        "SELECT MAX(sequence) FROM cloud_thread_rollout_items WHERE thread_id = ?",
    )
    .bind(thread_id.to_string())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_thread_error)?;
    let base_sequence = base_sequence.unwrap_or(0);
    let now_ms = now_millis();
    for (offset, item) in items.into_iter().enumerate() {
        let sequence = base_sequence + u64::try_from(offset).unwrap_or(u64::MAX) + 1;
        let item_json = serde_json::to_string(&item).map_err(serde_thread_store_error)?;
        sqlx::query(
            r#"INSERT INTO cloud_thread_rollout_items
(thread_id, sequence, item_json, created_at_ms)
VALUES (?, ?, ?, ?)"#,
        )
        .bind(thread_id.to_string())
        .bind(sequence)
        .bind(item_json)
        .bind(now_ms)
        .execute(&mut **tx)
        .await
        .map_err(storage_thread_error)?;
    }
    Ok(())
}

async fn ensure_thread_exists(pool: &MySqlPool, thread_id: ThreadId) -> ThreadStoreResult<()> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM cloud_threads WHERE thread_id = ?")
        .bind(thread_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(storage_thread_error)?;
    exists
        .is_some()
        .then_some(())
        .ok_or(ThreadStoreError::ThreadNotFound { thread_id })
}

async fn ensure_thread_exists_in_tx(
    tx: &mut Transaction<'_, MySql>,
    thread_id: ThreadId,
) -> ThreadStoreResult<()> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM cloud_threads WHERE thread_id = ?")
        .bind(thread_id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage_thread_error)?;
    exists
        .is_some()
        .then_some(())
        .ok_or(ThreadStoreError::ThreadNotFound { thread_id })
}

async fn set_archived_at(
    pool: MySqlPool,
    thread_id: ThreadId,
    archived_at_ms: Option<i64>,
) -> ThreadStoreResult<()> {
    let result = sqlx::query(
        "UPDATE cloud_threads SET archived_at_ms = ?, updated_at_ms = ? WHERE thread_id = ?",
    )
    .bind(archived_at_ms)
    .bind(now_millis())
    .bind(thread_id.to_string())
    .execute(&pool)
    .await
    .map_err(storage_thread_error)?;
    if result.rows_affected() == 0 {
        return Err(ThreadStoreError::ThreadNotFound { thread_id });
    }
    Ok(())
}

fn stored_thread_from_row(
    thread_id: ThreadId,
    create_params: CreateThreadParams,
    metadata_patch: Option<ThreadMetadataPatch>,
    created_at_ms: i64,
    updated_at_ms: i64,
    archived_at_ms: Option<i64>,
    history: Option<StoredThreadHistory>,
) -> StoredThread {
    let metadata_patch = metadata_patch.as_ref();
    StoredThread {
        thread_id,
        rollout_path: None,
        forked_from_id: create_params.forked_from_id,
        parent_thread_id: create_params.parent_thread_id,
        preview: metadata_patch
            .and_then(|metadata| metadata.preview.clone())
            .unwrap_or_default(),
        name: metadata_patch.and_then(|metadata| metadata.name.clone().flatten()),
        model_provider: metadata_patch
            .and_then(|metadata| metadata.model_provider.clone())
            .unwrap_or(create_params.metadata.model_provider),
        model: metadata_patch.and_then(|metadata| metadata.model.clone()),
        reasoning_effort: metadata_patch.and_then(|metadata| metadata.reasoning_effort),
        created_at: datetime_from_millis(created_at_ms),
        updated_at: datetime_from_millis(updated_at_ms),
        archived_at: archived_at_ms.map(datetime_from_millis),
        cwd: metadata_patch
            .and_then(|metadata| metadata.cwd.clone())
            .unwrap_or_else(|| create_params.metadata.cwd.unwrap_or_default()),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        source: create_params.source,
        thread_source: metadata_patch
            .and_then(|metadata| metadata.thread_source)
            .unwrap_or(create_params.thread_source),
        agent_nickname: None,
        agent_role: None,
        agent_path: None,
        git_info: None,
        approval_mode: metadata_patch
            .and_then(|metadata| metadata.approval_mode)
            .unwrap_or(AskForApproval::Never),
        permission_profile: metadata_patch
            .and_then(|metadata| metadata.permission_profile.clone())
            .unwrap_or_else(PermissionProfile::read_only),
        token_usage: metadata_patch.and_then(|metadata| metadata.token_usage.clone()),
        first_user_message: metadata_patch.and_then(|metadata| metadata.first_user_message.clone()),
        history,
    }
}

fn session_meta_line(params: &CreateThreadParams) -> SessionMetaLine {
    SessionMetaLine {
        meta: SessionMeta {
            id: params.thread_id,
            forked_from_id: params.forked_from_id,
            parent_thread_id: params.parent_thread_id,
            cwd: params.metadata.cwd.clone().unwrap_or_default(),
            source: params.source.clone(),
            thread_source: params.thread_source,
            model_provider: Some(params.metadata.model_provider.clone()),
            base_instructions: Some(params.base_instructions.clone()),
            dynamic_tools: (!params.dynamic_tools.is_empty()).then(|| params.dynamic_tools.clone()),
            memory_mode: matches!(params.metadata.memory_mode, ThreadMemoryMode::Disabled)
                .then_some("disabled".to_string()),
            multi_agent_version: params.multi_agent_version,
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            ..SessionMeta::default()
        },
        git: None,
    }
}

fn datetime_from_millis(ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_else(Utc::now)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn thread_id_from_string(value: String) -> ThreadStoreResult<ThreadId> {
    ThreadId::from_string(&value).map_err(|err| ThreadStoreError::Internal {
        message: format!("invalid stored thread id {value}: {err}"),
    })
}

fn serde_thread_store_error(err: serde_json::Error) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: format!("failed to serialize cloud thread store data: {err}"),
    }
}

fn storage_thread_error(err: sqlx::Error) -> ThreadStoreError {
    thread_store_error(storage_error(err))
}

fn thread_store_error(err: CloudStateError) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: err.to_string(),
    }
}
