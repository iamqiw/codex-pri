use codex_cloud_wrapper_protocol::RequestStatus;
use sqlx::Row;

use crate::CloudStateError;
use crate::EventRecord;
use crate::RequestRecord;
use crate::mysql_store::storage_error;

pub(super) fn request_from_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<RequestRecord, CloudStateError> {
    let status: String = row.try_get("status").map_err(storage_error)?;
    Ok(RequestRecord {
        request_id: row.try_get("request_id").map_err(storage_error)?,
        caller_id: row.try_get("caller_id").map_err(storage_error)?,
        thread_id: row.try_get("thread_id").map_err(storage_error)?,
        idempotency_key: row.try_get("idempotency_key").map_err(storage_error)?,
        input_hash: row.try_get("input_hash").map_err(storage_error)?,
        status: status_from_str(status.as_str())?,
        latest_event_cursor: row.try_get("latest_event_cursor").map_err(storage_error)?,
    })
}

pub(super) fn event_from_row(row: sqlx::mysql::MySqlRow) -> Result<EventRecord, CloudStateError> {
    Ok(EventRecord {
        request_id: row.try_get("request_id").map_err(storage_error)?,
        sequence: row.try_get("sequence").map_err(storage_error)?,
        event_type: row.try_get("event_type").map_err(storage_error)?,
        payload_inline: row.try_get("payload_inline").map_err(storage_error)?,
    })
}

pub(super) fn status_to_str(status: RequestStatus) -> &'static str {
    match status {
        RequestStatus::Queued => "queued",
        RequestStatus::Running => "running",
        RequestStatus::Cancelling => "cancelling",
        RequestStatus::Completed => "completed",
        RequestStatus::Failed => "failed",
        RequestStatus::Cancelled => "cancelled",
        RequestStatus::Interrupted => "interrupted",
        RequestStatus::OwnerTimedOut => "owner_timed_out",
        RequestStatus::LeaseLost => "lease_lost",
        RequestStatus::TurnTimedOut => "turn_timed_out",
    }
}

fn status_from_str(status: &str) -> Result<RequestStatus, CloudStateError> {
    match status {
        "queued" => Ok(RequestStatus::Queued),
        "running" => Ok(RequestStatus::Running),
        "cancelling" => Ok(RequestStatus::Cancelling),
        "completed" => Ok(RequestStatus::Completed),
        "failed" => Ok(RequestStatus::Failed),
        "cancelled" => Ok(RequestStatus::Cancelled),
        "interrupted" => Ok(RequestStatus::Interrupted),
        "owner_timed_out" => Ok(RequestStatus::OwnerTimedOut),
        "lease_lost" => Ok(RequestStatus::LeaseLost),
        "turn_timed_out" => Ok(RequestStatus::TurnTimedOut),
        _ => Err(CloudStateError::Storage(format!(
            "unknown request status: {status}"
        ))),
    }
}
