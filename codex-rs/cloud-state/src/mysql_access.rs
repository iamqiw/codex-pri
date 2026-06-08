use sqlx::MySqlPool;

use crate::CloudStateError;
use crate::mysql_store::storage_error;

pub(crate) async fn caller_can_access_thread(
    pool: &MySqlPool,
    caller_id: &str,
    thread_id: &str,
) -> Result<bool, CloudStateError> {
    let exists: Option<i64> = sqlx::query_scalar(
        r#"SELECT 1
FROM cloud_requests
WHERE caller_id = ? AND thread_id = ?
LIMIT 1"#,
    )
    .bind(caller_id)
    .bind(thread_id)
    .fetch_optional(pool)
    .await
    .map_err(storage_error)?;
    Ok(exists.is_some())
}
