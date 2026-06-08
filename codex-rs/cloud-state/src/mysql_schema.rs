use sqlx::MySqlPool;

use crate::CloudStateError;
use crate::mysql_store::storage_error;

pub struct MysqlCloudStateSchema;

impl MysqlCloudStateSchema {
    pub fn create_table_statements() -> [&'static str; 6] {
        [
            r#"CREATE TABLE IF NOT EXISTS cloud_requests (
    request_id VARCHAR(128) NOT NULL PRIMARY KEY,
    caller_id VARCHAR(255) NOT NULL,
    thread_id VARCHAR(255) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    input_hash TEXT NOT NULL,
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
    UNIQUE KEY cloud_thread_writer_leases_thread_id (thread_id),
    UNIQUE KEY cloud_thread_writer_leases_lease_id (lease_id)
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
    let exists: Option<i64> = sqlx::query_scalar(
        r#"SELECT 1
FROM information_schema.COLUMNS
WHERE TABLE_SCHEMA = DATABASE()
AND TABLE_NAME = 'cloud_append_results'
AND COLUMN_NAME = ?"#,
    )
    .bind(column_name)
    .fetch_optional(pool)
    .await
    .map_err(storage_error)?;
    Ok(exists.is_some())
}
