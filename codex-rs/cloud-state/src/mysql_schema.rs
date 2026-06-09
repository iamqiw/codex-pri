use sqlx::MySqlPool;

use crate::CloudStateError;
use crate::mysql_store::storage_error;

pub struct MysqlCloudStateSchema;

impl MysqlCloudStateSchema {
    pub fn create_table_statements() -> [&'static str; 11] {
        [
            r#"CREATE TABLE IF NOT EXISTS cloud_threads (
    thread_id VARCHAR(255) NOT NULL PRIMARY KEY,
    create_params_json JSON NOT NULL,
    metadata_patch_json JSON NULL,
    archived_at_ms BIGINT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_thread_rollout_items (
    thread_id VARCHAR(255) NOT NULL,
    sequence BIGINT UNSIGNED NOT NULL,
    item_json JSON NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (thread_id, sequence)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_requests (
    request_id VARCHAR(128) NOT NULL PRIMARY KEY,
    caller_id VARCHAR(255) NOT NULL,
    thread_id VARCHAR(255) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    input_hash TEXT NOT NULL,
    turn_id VARCHAR(255) NULL,
    status VARCHAR(64) NOT NULL,
    latest_event_cursor BIGINT UNSIGNED NOT NULL DEFAULT 0,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE KEY cloud_requests_idempotency_key (caller_id, thread_id, idempotency_key)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_thread_writer_leases (
    thread_id VARCHAR(255) NOT NULL PRIMARY KEY,
    request_id VARCHAR(128) NOT NULL,
    lease_id VARCHAR(128) NOT NULL,
    owner_instance_id VARCHAR(255) NOT NULL,
    fencing_token BIGINT UNSIGNED NOT NULL,
    acquired_at_ms BIGINT NOT NULL,
    heartbeat_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    released_at_ms BIGINT NULL,
    UNIQUE KEY cloud_thread_writer_leases_thread_id (thread_id),
    UNIQUE KEY cloud_thread_writer_leases_lease_id (lease_id)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_owner_leases (
    request_id VARCHAR(128) NOT NULL PRIMARY KEY,
    turn_id VARCHAR(255) NULL,
    owner_instance_id VARCHAR(255) NOT NULL,
    heartbeat_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    released_at_ms BIGINT NULL,
    KEY cloud_owner_leases_turn_id (turn_id)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_fencing_tokens (
    fencing_token BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
    request_id VARCHAR(128) NOT NULL,
    issued_at_ms BIGINT NOT NULL
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_append_results (
    thread_id VARCHAR(255) NOT NULL,
    request_id VARCHAR(128) NULL,
    append_idempotency_key VARCHAR(255) NOT NULL,
    payload_refs_json JSON NULL,
    thread_version BIGINT UNSIGNED NOT NULL,
    first_append_sequence BIGINT UNSIGNED NOT NULL,
    last_append_sequence BIGINT UNSIGNED NOT NULL,
    PRIMARY KEY (thread_id, append_idempotency_key),
    UNIQUE KEY cloud_append_results_idempotency_key (thread_id, append_idempotency_key)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_thread_items (
    thread_id VARCHAR(255) NOT NULL,
    sequence BIGINT UNSIGNED NOT NULL,
    payload_ref TEXT NOT NULL,
    PRIMARY KEY (thread_id, sequence)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_request_events (
    request_id VARCHAR(128) NOT NULL,
    sequence BIGINT UNSIGNED NOT NULL,
    event_type VARCHAR(255) NOT NULL,
    payload_inline JSON NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (request_id, sequence),
    UNIQUE KEY cloud_request_events_sequence (request_id, sequence)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_config_snapshots (
    request_id VARCHAR(128) NOT NULL PRIMARY KEY,
    thread_id VARCHAR(255) NOT NULL,
    config_json JSON NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    KEY cloud_config_snapshots_thread_id (thread_id)
)"#,
            r#"CREATE TABLE IF NOT EXISTS cloud_state_metadata (
    thread_id VARCHAR(255) NOT NULL,
    kind VARCHAR(255) NOT NULL,
    payload_json JSON NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (thread_id, kind)
)"#,
        ]
    }
}

pub(super) async fn create_schema(pool: &MySqlPool) -> Result<(), CloudStateError> {
    for statement in MysqlCloudStateSchema::create_table_statements() {
        sqlx::query(statement)
            .execute(pool)
            .await
            .map_err(storage_error)?;
    }
    migrate_append_results_request_id(pool).await?;
    migrate_append_results_payload_refs(pool).await?;
    migrate_requests_turn_id(pool).await?;
    migrate_thread_writer_lease_timestamps(pool).await?;
    migrate_owner_leases_turn_id(pool).await?;
    Ok(())
}

async fn migrate_append_results_request_id(pool: &MySqlPool) -> Result<(), CloudStateError> {
    if append_results_column_exists(pool, "request_id").await? {
        return Ok(());
    }
    sqlx::query("ALTER TABLE cloud_append_results ADD COLUMN request_id VARCHAR(128) NULL")
        .execute(pool)
        .await
        .map_err(storage_error)?;
    Ok(())
}

async fn migrate_append_results_payload_refs(pool: &MySqlPool) -> Result<(), CloudStateError> {
    if append_results_column_exists(pool, "payload_refs_json").await? {
        return Ok(());
    }
    sqlx::query("ALTER TABLE cloud_append_results ADD COLUMN payload_refs_json JSON NULL")
        .execute(pool)
        .await
        .map_err(storage_error)?;
    Ok(())
}

async fn append_results_column_exists(
    pool: &MySqlPool,
    column_name: &str,
) -> Result<bool, CloudStateError> {
    table_column_exists(pool, "cloud_append_results", column_name).await
}

async fn migrate_requests_turn_id(pool: &MySqlPool) -> Result<(), CloudStateError> {
    if table_column_exists(pool, "cloud_requests", "turn_id").await? {
        return Ok(());
    }
    sqlx::query("ALTER TABLE cloud_requests ADD COLUMN turn_id VARCHAR(255) NULL")
        .execute(pool)
        .await
        .map_err(storage_error)?;
    sqlx::query("CREATE INDEX cloud_requests_turn_id ON cloud_requests (turn_id)")
        .execute(pool)
        .await
        .map_err(storage_error)?;
    Ok(())
}

async fn migrate_thread_writer_lease_timestamps(pool: &MySqlPool) -> Result<(), CloudStateError> {
    if !table_column_exists(pool, "cloud_thread_writer_leases", "heartbeat_at_ms").await? {
        sqlx::query(
            "ALTER TABLE cloud_thread_writer_leases ADD COLUMN heartbeat_at_ms BIGINT NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await
        .map_err(storage_error)?;
    }
    if !table_column_exists(pool, "cloud_thread_writer_leases", "expires_at_ms").await? {
        sqlx::query(
            "ALTER TABLE cloud_thread_writer_leases ADD COLUMN expires_at_ms BIGINT NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await
        .map_err(storage_error)?;
    }
    if !table_column_exists(pool, "cloud_thread_writer_leases", "released_at_ms").await? {
        sqlx::query("ALTER TABLE cloud_thread_writer_leases ADD COLUMN released_at_ms BIGINT NULL")
            .execute(pool)
            .await
            .map_err(storage_error)?;
    }
    Ok(())
}

async fn migrate_owner_leases_turn_id(pool: &MySqlPool) -> Result<(), CloudStateError> {
    if table_column_exists(pool, "cloud_owner_leases", "turn_id").await? {
        return Ok(());
    }
    sqlx::query("ALTER TABLE cloud_owner_leases ADD COLUMN turn_id VARCHAR(255) NULL")
        .execute(pool)
        .await
        .map_err(storage_error)?;
    sqlx::query("CREATE INDEX cloud_owner_leases_turn_id ON cloud_owner_leases (turn_id)")
        .execute(pool)
        .await
        .map_err(storage_error)?;
    Ok(())
}

async fn table_column_exists(
    pool: &MySqlPool,
    table_name: &str,
    column_name: &str,
) -> Result<bool, CloudStateError> {
    let exists: Option<i64> = sqlx::query_scalar(
        r#"SELECT 1
FROM information_schema.COLUMNS
WHERE TABLE_SCHEMA = DATABASE()
AND TABLE_NAME = ?
AND COLUMN_NAME = ?"#,
    )
    .bind(table_name)
    .bind(column_name)
    .fetch_optional(pool)
    .await
    .map_err(storage_error)?;
    Ok(exists.is_some())
}
