use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseResponse;
use codex_cloud_wrapper_protocol::CloudRuntimeDefaults;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::ThreadItemRecord;
use codex_cloud_wrapper_protocol::WriterLeaseRecord;
use sqlx::MySqlPool;
use sqlx::Row;
use sqlx::mysql::MySqlPoolOptions;
use uuid::Uuid;

use crate::CloudStateError;
use crate::CloudStateStore;
use crate::ConfigSnapshotRecord;
use crate::CreateRequestParams;
use crate::EventAppendParams;
use crate::EventRecord;
use crate::EventsPage;
use crate::LeaseAcquireOutcome;
use crate::LeaseAppendParams;
use crate::RequestRecord;
use crate::StateMetadataRecord;
use crate::TerminalStatusUpdate;
use crate::mysql_access;
use crate::mysql_rows::event_from_row;
use crate::mysql_rows::request_from_row;
use crate::mysql_rows::status_to_str;
use crate::mysql_schema;
use crate::request_store::append_payload_refs_fingerprint;
use crate::request_store::append_payload_refs_match;
use crate::request_store::validate_event_payload_inline;

const THREAD_WRITER_LEASE_TTL_MS: i64 = 30_000;
const OWNER_LEASE_TTL_MS: i64 = 180_000;

#[derive(Debug, Clone)]
pub struct MysqlCloudStateStore {
    pub(crate) pool: MySqlPool,
}

#[derive(Debug, Clone)]
struct AppendResultRecord {
    request_id: String,
    response: AppendThreadItemsWithLeaseResponse,
    payload_refs_fingerprint: String,
}

impl MysqlCloudStateStore {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn from_database_url(database_url: &str) -> Result<Self, CloudStateError> {
        Self::from_database_url_with_max_connections(
            database_url,
            CloudRuntimeDefaults::default().mysql_max_connections,
        )
        .await
    }

    pub async fn from_database_url_with_max_connections(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, CloudStateError> {
        let pool = MySqlPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await
            .map_err(storage_error)?;
        let store = Self::new(pool);
        store.create_schema().await?;
        Ok(store)
    }

    pub fn from_database_url_lazy_with_max_connections(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, CloudStateError> {
        let pool = MySqlPoolOptions::new()
            .max_connections(max_connections)
            .connect_lazy(database_url)
            .map_err(storage_error)?;
        Ok(Self::new(pool))
    }

    pub async fn create_schema(&self) -> Result<(), CloudStateError> {
        mysql_schema::create_schema(&self.pool).await
    }

    #[doc(hidden)]
    pub async fn expire_thread_writer_lease_for_testing(
        &self,
        request_id: &str,
    ) -> Result<(), CloudStateError> {
        sqlx::query(
            "UPDATE cloud_thread_writer_leases SET heartbeat_at_ms = 0, expires_at_ms = 0 WHERE request_id = ?",
        )
        .bind(request_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }
}

impl CloudStateStore for MysqlCloudStateStore {
    async fn create_request(
        &mut self,
        params: CreateRequestParams,
    ) -> Result<RequestRecord, CloudStateError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if let Some(existing) = select_request_by_idempotency_key(
            &mut *tx,
            &params.caller_id,
            &params.thread_id,
            &params.idempotency_key,
        )
        .await?
        {
            if existing.input_hash != params.input_hash {
                return Err(CloudStateError::IdempotencyConflict);
            }
            tx.commit().await.map_err(storage_error)?;
            return Ok(existing);
        }

        let now_ms = now_millis();
        let request_id = format!("request-{}", Uuid::now_v7());
        let insert_result = sqlx::query(
            r#"INSERT INTO cloud_requests
(request_id, caller_id, thread_id, idempotency_key, input_hash, turn_id, status, latest_event_cursor, created_at_ms, updated_at_ms)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&request_id)
        .bind(&params.caller_id)
        .bind(&params.thread_id)
        .bind(&params.idempotency_key)
        .bind(&params.input_hash)
        .bind(None::<String>)
        .bind(status_to_str(RequestStatus::Queued))
        .bind(0_u64)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await;
        if let Err(err) = insert_result {
            if is_duplicate_entry(&err) {
                tx.rollback().await.map_err(storage_error)?;
                let existing = select_request_by_idempotency_key(
                    &self.pool,
                    &params.caller_id,
                    &params.thread_id,
                    &params.idempotency_key,
                )
                .await?
                .ok_or_else(|| {
                    CloudStateError::Storage(
                        "duplicate idempotency key was reported but existing request was not found"
                            .to_string(),
                    )
                })?;
                if existing.input_hash != params.input_hash {
                    return Err(CloudStateError::IdempotencyConflict);
                }
                return Ok(existing);
            }
            return Err(storage_error(err));
        }
        tx.commit().await.map_err(storage_error)?;

        Ok(RequestRecord {
            request_id,
            caller_id: params.caller_id,
            thread_id: params.thread_id,
            idempotency_key: params.idempotency_key,
            input_hash: params.input_hash,
            turn_id: None,
            status: RequestStatus::Queued,
            latest_event_cursor: 0,
        })
    }

    async fn read_request(&self, request_id: &str) -> Result<RequestRecord, CloudStateError> {
        select_request_by_id(&self.pool, request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)
    }

    async fn read_request_by_turn_id(
        &self,
        turn_id: &str,
    ) -> Result<RequestRecord, CloudStateError> {
        select_request_by_turn_id(&self.pool, turn_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)
    }

    async fn set_request_turn_id(
        &mut self,
        request_id: &str,
        turn_id: &str,
    ) -> Result<RequestRecord, CloudStateError> {
        let result = sqlx::query(
            "UPDATE cloud_requests SET turn_id = ?, updated_at_ms = ? WHERE request_id = ?",
        )
        .bind(turn_id)
        .bind(now_millis())
        .bind(request_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        if result.rows_affected() == 0 {
            return Err(CloudStateError::RequestNotFound);
        }
        sqlx::query("UPDATE cloud_owner_leases SET turn_id = ? WHERE request_id = ?")
            .bind(turn_id)
            .bind(request_id)
            .execute(&self.pool)
            .await
            .map_err(storage_error)?;
        self.read_request(request_id).await
    }

    async fn caller_can_access_thread(
        &self,
        caller_id: &str,
        thread_id: &str,
    ) -> Result<bool, CloudStateError> {
        mysql_access::caller_can_access_thread(&self.pool, caller_id, thread_id).await
    }

    async fn acquire_thread_writer_lease(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<LeaseAcquireOutcome, CloudStateError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let request = select_request_by_id_for_update(&mut *tx, request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)?;
        let now_ms = now_millis();
        let existing_lease: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT request_id, lease_id, expires_at_ms FROM cloud_thread_writer_leases WHERE thread_id = ? FOR UPDATE",
        )
        .bind(&request.thread_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        if let Some((lease_request_id, _lease_id, expires_at_ms)) = existing_lease {
            if expires_at_ms <= now_ms {
                mark_request_terminal_event_locked(
                    &mut tx,
                    &lease_request_id,
                    RequestStatus::LeaseLost,
                    "request/lease_lost",
                    r#"{"reason":"thread_writer_lease_expired"}"#,
                    now_ms,
                )
                .await?;
                release_thread_and_owner_leases_locked(
                    &mut tx,
                    &request.thread_id,
                    &lease_request_id,
                    now_ms,
                )
                .await?;
            } else {
                tx.commit().await.map_err(storage_error)?;
                return Ok(LeaseAcquireOutcome::Queued);
            }
        }
        let request = select_request_by_id_for_update(&mut *tx, request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)?;
        if request.status != RequestStatus::Queued {
            tx.commit().await.map_err(storage_error)?;
            return Err(CloudStateError::RequestNotQueued);
        }

        let lease_id = format!("lease-{}", Uuid::now_v7());
        sqlx::query("INSERT INTO cloud_fencing_tokens (request_id, issued_at_ms) VALUES (?, ?)")
            .bind(&request.request_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(storage_error)?;
        let fencing_token: u64 = sqlx::query_scalar("SELECT CAST(LAST_INSERT_ID() AS UNSIGNED)")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage_error)?;
        let insert_result = sqlx::query(
            r#"INSERT INTO cloud_thread_writer_leases
(thread_id, request_id, lease_id, owner_instance_id, fencing_token, acquired_at_ms, heartbeat_at_ms, expires_at_ms, released_at_ms)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&request.thread_id)
        .bind(&request.request_id)
        .bind(&lease_id)
        .bind(owner_instance_id)
        .bind(fencing_token)
        .bind(now_ms)
        .bind(now_ms)
        .bind(now_ms + THREAD_WRITER_LEASE_TTL_MS)
        .bind(None::<i64>)
        .execute(&mut *tx)
        .await;
        if let Err(err) = insert_result {
            if is_duplicate_entry(&err) {
                tx.rollback().await.map_err(storage_error)?;
                return Ok(LeaseAcquireOutcome::Queued);
            }
            return Err(storage_error(err));
        }
        sqlx::query(
            r#"INSERT INTO cloud_owner_leases
(request_id, turn_id, owner_instance_id, heartbeat_at_ms, expires_at_ms, released_at_ms)
VALUES (?, ?, ?, ?, ?, ?)
ON DUPLICATE KEY UPDATE
    owner_instance_id = VALUES(owner_instance_id),
    heartbeat_at_ms = VALUES(heartbeat_at_ms),
    expires_at_ms = VALUES(expires_at_ms),
    released_at_ms = NULL"#,
        )
        .bind(&request.request_id)
        .bind(request.turn_id.as_deref())
        .bind(owner_instance_id)
        .bind(now_ms)
        .bind(now_ms + OWNER_LEASE_TTL_MS)
        .bind(None::<i64>)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        update_request_status(&mut *tx, &request.request_id, RequestStatus::Running).await?;
        tx.commit().await.map_err(storage_error)?;

        Ok(LeaseAcquireOutcome::Acquired {
            lease_id,
            fencing_token,
        })
    }

    async fn read_thread_writer_lease(
        &self,
        request_id: &str,
    ) -> Result<Option<WriterLeaseRecord>, CloudStateError> {
        self.read_request(request_id).await?;
        let lease: Option<(String, String, u64)> = sqlx::query_as(
            r#"SELECT lease_id, owner_instance_id, fencing_token
FROM cloud_thread_writer_leases
WHERE request_id = ? AND released_at_ms IS NULL AND expires_at_ms > ?"#,
        )
        .bind(request_id)
        .bind(now_millis())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(lease.map(
            |(lease_id, writer_owner_token, fencing_token)| WriterLeaseRecord {
                lease_id,
                writer_owner_token,
                fencing_token,
            },
        ))
    }

    async fn renew_request_leases(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<bool, CloudStateError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let request = select_request_by_id_for_update(&mut *tx, request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)?;
        if request.status.is_terminal() {
            tx.commit().await.map_err(storage_error)?;
            return Ok(false);
        }
        let now_ms = now_millis();
        let writer_result = sqlx::query(
            r#"UPDATE cloud_thread_writer_leases
SET heartbeat_at_ms = ?, expires_at_ms = ?
WHERE thread_id = ?
  AND request_id = ?
  AND owner_instance_id = ?
  AND released_at_ms IS NULL
  AND expires_at_ms > ?"#,
        )
        .bind(now_ms)
        .bind(now_ms + THREAD_WRITER_LEASE_TTL_MS)
        .bind(&request.thread_id)
        .bind(&request.request_id)
        .bind(owner_instance_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        if writer_result.rows_affected() == 0 {
            mark_request_terminal_event_locked(
                &mut tx,
                &request.request_id,
                RequestStatus::LeaseLost,
                "request/lease_lost",
                r#"{"reason":"thread_writer_lease_lost"}"#,
                now_ms,
            )
            .await?;
            release_thread_and_owner_leases_locked(
                &mut tx,
                &request.thread_id,
                &request.request_id,
                now_ms,
            )
            .await?;
            tx.commit().await.map_err(storage_error)?;
            return Ok(false);
        }

        let owner_result = sqlx::query(
            r#"UPDATE cloud_owner_leases
SET heartbeat_at_ms = ?, expires_at_ms = ?
WHERE request_id = ?
  AND owner_instance_id = ?
  AND released_at_ms IS NULL
  AND expires_at_ms > ?"#,
        )
        .bind(now_ms)
        .bind(now_ms + OWNER_LEASE_TTL_MS)
        .bind(&request.request_id)
        .bind(owner_instance_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        if owner_result.rows_affected() == 0 {
            mark_request_terminal_event_locked(
                &mut tx,
                &request.request_id,
                RequestStatus::OwnerTimedOut,
                "request/owner_timed_out",
                r#"{"reason":"owner_lease_lost"}"#,
                now_ms,
            )
            .await?;
            release_thread_and_owner_leases_locked(
                &mut tx,
                &request.thread_id,
                &request.request_id,
                now_ms,
            )
            .await?;
            tx.commit().await.map_err(storage_error)?;
            return Ok(false);
        }

        tx.commit().await.map_err(storage_error)?;
        Ok(true)
    }

    async fn expire_owner_leases(&mut self) -> Result<usize, CloudStateError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let now_ms = now_millis();
        let expired_requests: Vec<(String, String)> = sqlx::query_as(
            r#"SELECT r.request_id, r.thread_id
FROM cloud_requests r
JOIN cloud_owner_leases o ON o.request_id = r.request_id
WHERE r.status = 'running'
  AND o.released_at_ms IS NULL
  AND o.expires_at_ms <= ?
FOR UPDATE"#,
        )
        .bind(now_ms)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let mut expired_count = 0;
        for (request_id, thread_id) in expired_requests {
            let update = mark_request_terminal_event_locked(
                &mut tx,
                &request_id,
                RequestStatus::OwnerTimedOut,
                "request/owner_timed_out",
                r#"{"reason":"owner_lease_expired"}"#,
                now_ms,
            )
            .await?;
            release_thread_and_owner_leases_locked(&mut tx, &thread_id, &request_id, now_ms)
                .await?;
            if update == TerminalStatusUpdate::Changed {
                expired_count += 1;
            }
        }
        tx.commit().await.map_err(storage_error)?;
        Ok(expired_count)
    }

    async fn append_thread_items_with_lease(
        &mut self,
        params: LeaseAppendParams,
    ) -> Result<AppendThreadItemsWithLeaseResponse, CloudStateError> {
        if params.payload_refs.is_empty() {
            return Err(CloudStateError::EmptyAppend);
        }

        let request = select_request_by_id(&self.pool, &params.request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let now_ms = now_millis();
        let lease: Option<(String, String, String, u64, i64)> = sqlx::query_as(
            "SELECT request_id, lease_id, owner_instance_id, fencing_token, expires_at_ms FROM cloud_thread_writer_leases WHERE thread_id = ? AND released_at_ms IS NULL FOR UPDATE",
        )
        .bind(&request.thread_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let release_lease = lease
            .as_ref()
            .is_some_and(|(lease_request_id, _, _, _, _)| lease_request_id == &params.request_id);
        let valid_lease = lease.is_some_and(
            |(lease_request_id, lease_id, owner_instance_id, fencing_token, expires_at_ms)| {
                lease_request_id == params.request_id
                    && lease_id == params.lease_id
                    && owner_instance_id == params.writer_owner_token
                    && fencing_token == params.fencing_token
                    && expires_at_ms > now_ms
            },
        );
        if !valid_lease {
            if release_lease {
                mark_request_terminal_event_locked(
                    &mut tx,
                    &params.request_id,
                    RequestStatus::LeaseLost,
                    "request/lease_lost",
                    r#"{"reason":"thread_writer_lease_lost"}"#,
                    now_ms,
                )
                .await?;
                release_thread_and_owner_leases_locked(
                    &mut tx,
                    &request.thread_id,
                    &params.request_id,
                    now_ms,
                )
                .await?;
            }
            tx.commit().await.map_err(storage_error)?;
            return Err(CloudStateError::LeaseLost);
        }

        if let Some(existing) =
            select_append_result(&mut *tx, &request.thread_id, &params.append_idempotency_key)
                .await?
        {
            if existing.request_id != params.request_id {
                tx.rollback().await.map_err(storage_error)?;
                return Err(CloudStateError::IdempotencyConflict);
            }
            if !append_payload_refs_match(&existing.payload_refs_fingerprint, &params.payload_refs)
            {
                tx.rollback().await.map_err(storage_error)?;
                return Err(CloudStateError::IdempotencyConflict);
            }
            tx.commit().await.map_err(storage_error)?;
            return Ok(AppendThreadItemsWithLeaseResponse {
                deduplicated: true,
                ..existing.response
            });
        }

        let payload_refs_fingerprint = append_payload_refs_fingerprint(&params.payload_refs)?;
        let current_version: u64 = sqlx::query_scalar(
            "SELECT CAST(COALESCE(MAX(last_append_sequence), 0) AS UNSIGNED) FROM cloud_append_results WHERE thread_id = ?",
        )
        .bind(&request.thread_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
        if params
            .expected_thread_version
            .is_some_and(|expected| expected != current_version)
        {
            tx.rollback().await.map_err(storage_error)?;
            return Err(CloudStateError::ThreadVersionConflict);
        }
        let item_count = params.payload_refs.len() as u64;
        let next_version = current_version + item_count;
        let response = AppendThreadItemsWithLeaseResponse {
            thread_version: next_version,
            first_append_sequence: current_version + 1,
            last_append_sequence: next_version,
            deduplicated: false,
        };
        for (index, payload_ref) in params.payload_refs.iter().enumerate() {
            sqlx::query(
                r#"INSERT INTO cloud_thread_items
(thread_id, sequence, payload_ref)
VALUES (?, ?, ?)"#,
            )
            .bind(&request.thread_id)
            .bind(current_version + index as u64 + 1)
            .bind(payload_ref)
            .execute(&mut *tx)
            .await
            .map_err(storage_error)?;
        }
        let insert_result = sqlx::query(
            r#"INSERT INTO cloud_append_results
(thread_id, request_id, append_idempotency_key, payload_refs_json, thread_version, first_append_sequence, last_append_sequence)
VALUES (?, ?, ?, CAST(? AS JSON), ?, ?, ?)"#,
        )
        .bind(&request.thread_id)
        .bind(&params.request_id)
        .bind(&params.append_idempotency_key)
        .bind(&payload_refs_fingerprint)
        .bind(response.thread_version)
        .bind(response.first_append_sequence)
        .bind(response.last_append_sequence)
        .execute(&mut *tx)
        .await;
        if let Err(err) = insert_result {
            if is_duplicate_entry(&err) {
                tx.rollback().await.map_err(storage_error)?;
                let existing =
                    select_append_result(&self.pool, &request.thread_id, &params.append_idempotency_key)
                        .await?
                        .ok_or_else(|| {
                            CloudStateError::Storage(
                                "duplicate append key was reported but existing append result was not found"
                                    .to_string(),
                            )
                        })?;
                if existing.request_id != params.request_id {
                    return Err(CloudStateError::IdempotencyConflict);
                }
                if !append_payload_refs_match(
                    &existing.payload_refs_fingerprint,
                    &params.payload_refs,
                ) {
                    return Err(CloudStateError::IdempotencyConflict);
                }
                return Ok(AppendThreadItemsWithLeaseResponse {
                    deduplicated: true,
                    ..existing.response
                });
            }
            return Err(storage_error(err));
        }
        tx.commit().await.map_err(storage_error)?;

        Ok(response)
    }

    async fn list_thread_items(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<ThreadItemRecord>, Option<u64>), CloudStateError> {
        let after = cursor.unwrap_or(0);
        let rows = sqlx::query(
            r#"SELECT thread_id, sequence, payload_ref
FROM cloud_thread_items
WHERE thread_id = ? AND sequence > ?
ORDER BY sequence ASC
LIMIT ?"#,
        )
        .bind(thread_id)
        .bind(after)
        .bind(limit as u64 + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;

        let has_more = rows.len() > limit;
        let data = rows
            .into_iter()
            .take(limit)
            .map(|row| {
                Ok(ThreadItemRecord {
                    thread_id: row.try_get("thread_id").map_err(storage_error)?,
                    sequence: row.try_get("sequence").map_err(storage_error)?,
                    payload_ref: row.try_get("payload_ref").map_err(storage_error)?,
                })
            })
            .collect::<Result<Vec<_>, CloudStateError>>()?;
        let next_cursor = if has_more {
            data.last().map(|item| item.sequence)
        } else {
            None
        };

        Ok((data, next_cursor))
    }

    async fn mark_request_terminal(
        &mut self,
        request_id: &str,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        debug_assert!(status.is_terminal());
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let now_ms = now_millis();
        let request = select_request_by_id_for_update(&mut *tx, request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)?;
        if request.status.is_terminal() {
            tx.commit().await.map_err(storage_error)?;
            return Ok(TerminalStatusUpdate::Unchanged);
        }
        let status = if terminal_requires_active_lease(status)
            && request.status == RequestStatus::Running
            && !request_has_active_lease(&mut tx, &request, now_ms).await?
        {
            RequestStatus::LeaseLost
        } else {
            status
        };
        let result = sqlx::query(
            r#"UPDATE cloud_requests
SET status = ?, updated_at_ms = ?
WHERE request_id = ? AND status IN ('queued', 'running', 'cancelling')"#,
        )
        .bind(status_to_str(status))
        .bind(now_ms)
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        let update = if result.rows_affected() == 0 {
            TerminalStatusUpdate::Unchanged
        } else {
            TerminalStatusUpdate::Changed
        };
        release_thread_and_owner_leases_locked(&mut tx, &request.thread_id, request_id, now_ms)
            .await?;
        tx.commit().await.map_err(storage_error)?;
        Ok(update)
    }

    async fn mark_request_terminal_with_event(
        &mut self,
        params: EventAppendParams,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        debug_assert!(status.is_terminal());
        validate_event_payload_inline(&params.payload_inline)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let now_ms = now_millis();
        let request = select_request_by_id_for_update(&mut *tx, &params.request_id)
            .await?
            .ok_or(CloudStateError::RequestNotFound)?;
        if request.status.is_terminal() {
            tx.commit().await.map_err(storage_error)?;
            return Ok(TerminalStatusUpdate::Unchanged);
        }
        if terminal_requires_active_lease(status)
            && request.status == RequestStatus::Running
            && !request_has_active_lease(&mut tx, &request, now_ms).await?
        {
            mark_request_terminal_event_locked(
                &mut tx,
                &params.request_id,
                RequestStatus::LeaseLost,
                "request/lease_lost",
                r#"{"reason":"thread_writer_lease_lost"}"#,
                now_ms,
            )
            .await?;
            release_thread_and_owner_leases_locked(
                &mut tx,
                &request.thread_id,
                &params.request_id,
                now_ms,
            )
            .await?;
            tx.commit().await.map_err(storage_error)?;
            return Ok(TerminalStatusUpdate::Changed);
        }
        let result = sqlx::query(
            r#"UPDATE cloud_requests
SET status = ?, latest_event_cursor = latest_event_cursor + 1, updated_at_ms = ?
WHERE request_id = ? AND status IN ('queued', 'running', 'cancelling')"#,
        )
        .bind(status_to_str(status))
        .bind(now_ms)
        .bind(&params.request_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        release_thread_and_owner_leases_locked(
            &mut tx,
            &request.thread_id,
            &params.request_id,
            now_ms,
        )
        .await?;
        if result.rows_affected() == 0 {
            tx.commit().await.map_err(storage_error)?;
            return Ok(TerminalStatusUpdate::Unchanged);
        }
        sqlx::query(
            r#"INSERT INTO cloud_request_events
(request_id, sequence, event_type, payload_inline, created_at_ms)
VALUES (?, ?, ?, CAST(? AS JSON), ?)"#,
        )
        .bind(&params.request_id)
        .bind(request.latest_event_cursor + 1)
        .bind(&params.event_type)
        .bind(&params.payload_inline)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(TerminalStatusUpdate::Changed)
    }

    async fn append_request_event(
        &mut self,
        params: EventAppendParams,
    ) -> Result<EventRecord, CloudStateError> {
        validate_event_payload_inline(&params.payload_inline)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let now_ms = now_millis();
        let result = sqlx::query(
            r#"UPDATE cloud_requests
SET latest_event_cursor = latest_event_cursor + 1, updated_at_ms = ?
WHERE request_id = ?"#,
        )
        .bind(now_ms)
        .bind(&params.request_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        if result.rows_affected() == 0 {
            return Err(CloudStateError::RequestNotFound);
        }
        let sequence: u64 = sqlx::query_scalar(
            "SELECT latest_event_cursor FROM cloud_requests WHERE request_id = ?",
        )
        .bind(&params.request_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
        sqlx::query(
            r#"INSERT INTO cloud_request_events
(request_id, sequence, event_type, payload_inline, created_at_ms)
VALUES (?, ?, ?, CAST(? AS JSON), ?)"#,
        )
        .bind(&params.request_id)
        .bind(sequence)
        .bind(&params.event_type)
        .bind(&params.payload_inline)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;

        Ok(EventRecord {
            request_id: params.request_id,
            sequence,
            event_type: params.event_type,
            payload_inline: params.payload_inline,
        })
    }

    async fn append_request_event_if_latest_cursor(
        &mut self,
        params: EventAppendParams,
        expected_latest_event_cursor: u64,
    ) -> Result<Option<EventRecord>, CloudStateError> {
        validate_event_payload_inline(&params.payload_inline)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let now_ms = now_millis();
        let result = sqlx::query(
            r#"UPDATE cloud_requests
SET latest_event_cursor = latest_event_cursor + 1, updated_at_ms = ?
WHERE request_id = ? AND latest_event_cursor = ?"#,
        )
        .bind(now_ms)
        .bind(&params.request_id)
        .bind(expected_latest_event_cursor)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        if result.rows_affected() == 0 {
            select_request_by_id(&mut *tx, &params.request_id)
                .await?
                .ok_or(CloudStateError::RequestNotFound)?;
            tx.commit().await.map_err(storage_error)?;
            return Ok(None);
        }
        let sequence = expected_latest_event_cursor + 1;
        sqlx::query(
            r#"INSERT INTO cloud_request_events
(request_id, sequence, event_type, payload_inline, created_at_ms)
VALUES (?, ?, ?, CAST(? AS JSON), ?)"#,
        )
        .bind(&params.request_id)
        .bind(sequence)
        .bind(&params.event_type)
        .bind(&params.payload_inline)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;

        Ok(Some(EventRecord {
            request_id: params.request_id,
            sequence,
            event_type: params.event_type,
            payload_inline: params.payload_inline,
        }))
    }

    async fn list_request_events(
        &self,
        request_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<EventsPage, CloudStateError> {
        self.read_request(request_id).await?;
        let after = cursor.unwrap_or(0);
        let rows = sqlx::query(
            r#"SELECT request_id, sequence, event_type, JSON_UNQUOTE(payload_inline) AS payload_inline
FROM cloud_request_events
WHERE request_id = ? AND sequence > ?
ORDER BY sequence ASC
LIMIT ?"#,
        )
        .bind(request_id)
        .bind(after)
        .bind(limit as u64 + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        let has_more = rows.len() > limit;
        let data = rows
            .into_iter()
            .take(limit)
            .map(event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if has_more {
            data.last().map(|event| event.sequence)
        } else {
            None
        };

        Ok(EventsPage { data, next_cursor })
    }

    async fn persist_config_snapshot(
        &mut self,
        record: ConfigSnapshotRecord,
    ) -> Result<(), CloudStateError> {
        validate_json_payload(&record.config_json)?;
        let request = self.read_request(&record.request_id).await?;
        if request.thread_id != record.thread_id {
            return Err(CloudStateError::ThreadMismatch);
        }
        let now_ms = now_millis();
        sqlx::query(
            r#"INSERT INTO cloud_config_snapshots
(request_id, thread_id, config_json, created_at_ms, updated_at_ms)
VALUES (?, ?, CAST(? AS JSON), ?, ?)
ON DUPLICATE KEY UPDATE
    thread_id = VALUES(thread_id),
    config_json = VALUES(config_json),
    updated_at_ms = VALUES(updated_at_ms)"#,
        )
        .bind(&record.request_id)
        .bind(&record.thread_id)
        .bind(&record.config_json)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn read_config_snapshot(
        &self,
        request_id: &str,
    ) -> Result<ConfigSnapshotRecord, CloudStateError> {
        self.read_request(request_id).await?;
        let row = sqlx::query(
            r#"SELECT request_id, thread_id, JSON_UNQUOTE(config_json) AS config_json
FROM cloud_config_snapshots
WHERE request_id = ?"#,
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .ok_or(CloudStateError::RequestNotFound)?;
        Ok(ConfigSnapshotRecord {
            request_id: row.try_get("request_id").map_err(storage_error)?,
            thread_id: row.try_get("thread_id").map_err(storage_error)?,
            config_json: row.try_get("config_json").map_err(storage_error)?,
        })
    }

    async fn upsert_state_metadata(
        &mut self,
        record: StateMetadataRecord,
    ) -> Result<(), CloudStateError> {
        validate_json_payload(&record.payload_json)?;
        let now_ms = now_millis();
        sqlx::query(
            r#"INSERT INTO cloud_state_metadata
(thread_id, kind, payload_json, created_at_ms, updated_at_ms)
VALUES (?, ?, CAST(? AS JSON), ?, ?)
ON DUPLICATE KEY UPDATE
    payload_json = VALUES(payload_json),
    updated_at_ms = VALUES(updated_at_ms)"#,
        )
        .bind(&record.thread_id)
        .bind(&record.kind)
        .bind(&record.payload_json)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn read_state_metadata(
        &self,
        thread_id: &str,
        kind: &str,
    ) -> Result<StateMetadataRecord, CloudStateError> {
        let row = sqlx::query(
            r#"SELECT thread_id, kind, JSON_UNQUOTE(payload_json) AS payload_json
FROM cloud_state_metadata
WHERE thread_id = ? AND kind = ?"#,
        )
        .bind(thread_id)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .ok_or(CloudStateError::RequestNotFound)?;
        Ok(StateMetadataRecord {
            thread_id: row.try_get("thread_id").map_err(storage_error)?,
            kind: row.try_get("kind").map_err(storage_error)?,
            payload_json: row.try_get("payload_json").map_err(storage_error)?,
        })
    }
}

async fn select_request_by_id<'e, E>(
    executor: E,
    request_id: &str,
) -> Result<Option<RequestRecord>, CloudStateError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let row = sqlx::query(
        r#"SELECT request_id, caller_id, thread_id, idempotency_key, input_hash, turn_id, status, latest_event_cursor
FROM cloud_requests
WHERE request_id = ?"#,
    )
    .bind(request_id)
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(request_from_row).transpose()
}

async fn select_request_by_id_for_update<'e, E>(
    executor: E,
    request_id: &str,
) -> Result<Option<RequestRecord>, CloudStateError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let row = sqlx::query(
        r#"SELECT request_id, caller_id, thread_id, idempotency_key, input_hash, turn_id, status, latest_event_cursor
FROM cloud_requests
WHERE request_id = ?
FOR UPDATE"#,
    )
    .bind(request_id)
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(request_from_row).transpose()
}

async fn select_request_by_idempotency_key<'e, E>(
    executor: E,
    caller_id: &str,
    thread_id: &str,
    idempotency_key: &str,
) -> Result<Option<RequestRecord>, CloudStateError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let row = sqlx::query(
        r#"SELECT request_id, caller_id, thread_id, idempotency_key, input_hash, turn_id, status, latest_event_cursor
FROM cloud_requests
WHERE caller_id = ? AND thread_id = ? AND idempotency_key = ?"#,
    )
    .bind(caller_id)
    .bind(thread_id)
    .bind(idempotency_key)
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(request_from_row).transpose()
}

async fn select_request_by_turn_id<'e, E>(
    executor: E,
    turn_id: &str,
) -> Result<Option<RequestRecord>, CloudStateError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let row = sqlx::query(
        r#"SELECT request_id, caller_id, thread_id, idempotency_key, input_hash, turn_id, status, latest_event_cursor
FROM cloud_requests
WHERE turn_id = ?"#,
    )
    .bind(turn_id)
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(request_from_row).transpose()
}

async fn select_append_result<'e, E>(
    executor: E,
    thread_id: &str,
    append_idempotency_key: &str,
) -> Result<Option<AppendResultRecord>, CloudStateError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let row = sqlx::query(
        r#"SELECT COALESCE(request_id, '') AS request_id,
COALESCE(JSON_UNQUOTE(payload_refs_json), '') AS payload_refs_json,
thread_version, first_append_sequence, last_append_sequence
FROM cloud_append_results
WHERE thread_id = ? AND append_idempotency_key = ?"#,
    )
    .bind(thread_id)
    .bind(append_idempotency_key)
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(AppendResultRecord {
            request_id: row.try_get("request_id").map_err(storage_error)?,
            response: AppendThreadItemsWithLeaseResponse {
                thread_version: row.try_get("thread_version").map_err(storage_error)?,
                first_append_sequence: row
                    .try_get("first_append_sequence")
                    .map_err(storage_error)?,
                last_append_sequence: row.try_get("last_append_sequence").map_err(storage_error)?,
                deduplicated: false,
            },
            payload_refs_fingerprint: row.try_get("payload_refs_json").map_err(storage_error)?,
        })
    })
    .transpose()
}

fn terminal_requires_active_lease(status: RequestStatus) -> bool {
    matches!(
        status,
        RequestStatus::Completed | RequestStatus::Failed | RequestStatus::Interrupted
    )
}

fn validate_json_payload(payload_inline: &str) -> Result<(), CloudStateError> {
    serde_json::from_str::<serde_json::Value>(payload_inline)
        .map(|_| ())
        .map_err(|_| CloudStateError::InvalidEventPayload)
}

async fn request_has_active_lease(
    executor: &mut sqlx::MySqlConnection,
    request: &RequestRecord,
    now_ms: i64,
) -> Result<bool, CloudStateError> {
    let active_lease: Option<(String,)> = sqlx::query_as(
        r#"SELECT lease_id
FROM cloud_thread_writer_leases
WHERE thread_id = ?
  AND request_id = ?
  AND released_at_ms IS NULL
  AND expires_at_ms > ?
FOR UPDATE"#,
    )
    .bind(&request.thread_id)
    .bind(&request.request_id)
    .bind(now_ms)
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(active_lease.is_some())
}

async fn mark_request_terminal_event_locked(
    executor: &mut sqlx::MySqlConnection,
    request_id: &str,
    status: RequestStatus,
    event_type: &str,
    payload_inline: &str,
    now_ms: i64,
) -> Result<TerminalStatusUpdate, CloudStateError> {
    validate_event_payload_inline(payload_inline)?;
    let result = sqlx::query(
        r#"UPDATE cloud_requests
SET status = ?, latest_event_cursor = latest_event_cursor + 1, updated_at_ms = ?
WHERE request_id = ? AND status IN ('queued', 'running', 'cancelling')"#,
    )
    .bind(status_to_str(status))
    .bind(now_ms)
    .bind(request_id)
    .execute(&mut *executor)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() == 0 {
        return Ok(TerminalStatusUpdate::Unchanged);
    }
    let latest_event_cursor: u64 =
        sqlx::query_scalar("SELECT latest_event_cursor FROM cloud_requests WHERE request_id = ?")
            .bind(request_id)
            .fetch_one(&mut *executor)
            .await
            .map_err(storage_error)?;
    sqlx::query(
        r#"INSERT INTO cloud_request_events
(request_id, sequence, event_type, payload_inline, created_at_ms)
VALUES (?, ?, ?, CAST(? AS JSON), ?)"#,
    )
    .bind(request_id)
    .bind(latest_event_cursor)
    .bind(event_type)
    .bind(payload_inline)
    .bind(now_ms)
    .execute(&mut *executor)
    .await
    .map_err(storage_error)?;
    Ok(TerminalStatusUpdate::Changed)
}

async fn release_thread_and_owner_leases_locked(
    executor: &mut sqlx::MySqlConnection,
    thread_id: &str,
    request_id: &str,
    now_ms: i64,
) -> Result<(), CloudStateError> {
    sqlx::query(
        r#"UPDATE cloud_thread_writer_leases
SET released_at_ms = ?
WHERE thread_id = ? AND request_id = ? AND released_at_ms IS NULL"#,
    )
    .bind(now_ms)
    .bind(thread_id)
    .bind(request_id)
    .execute(&mut *executor)
    .await
    .map_err(storage_error)?;
    sqlx::query("DELETE FROM cloud_thread_writer_leases WHERE thread_id = ? AND request_id = ?")
        .bind(thread_id)
        .bind(request_id)
        .execute(&mut *executor)
        .await
        .map_err(storage_error)?;
    sqlx::query(
        r#"UPDATE cloud_owner_leases
SET released_at_ms = ?
WHERE request_id = ? AND released_at_ms IS NULL"#,
    )
    .bind(now_ms)
    .bind(request_id)
    .execute(&mut *executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn update_request_status<'e, E>(
    executor: E,
    request_id: &str,
    status: RequestStatus,
) -> Result<(), CloudStateError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let status = status_to_str(status);
    let result = sqlx::query(
        r#"UPDATE cloud_requests
SET status = CASE
    WHEN status IN ('completed', 'failed', 'cancelled', 'interrupted', 'owner_timed_out', 'lease_lost', 'turn_timed_out') AND status <> ? THEN status
    ELSE ?
END,
updated_at_ms = ?
WHERE request_id = ?"#,
    )
    .bind(status)
    .bind(status)
    .bind(now_millis())
    .bind(request_id)
    .execute(executor)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() == 0 {
        return Err(CloudStateError::RequestNotFound);
    }
    Ok(())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

pub(crate) fn storage_error(err: sqlx::Error) -> CloudStateError {
    CloudStateError::Storage(err.to_string())
}

fn is_duplicate_entry(err: &sqlx::Error) -> bool {
    err.as_database_error().is_some_and(|database_error| {
        database_error
            .code()
            .is_some_and(|code| code == "1062" || code == "23000")
            || database_error.message().contains("Duplicate entry")
    })
}
