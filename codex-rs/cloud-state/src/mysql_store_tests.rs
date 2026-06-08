use codex_cloud_wrapper_protocol::AppendThreadItem;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseParams;
use codex_cloud_wrapper_protocol::RequestCancelParams;
use codex_cloud_wrapper_protocol::RequestEventsListParams;
use codex_cloud_wrapper_protocol::RequestReadParams;
use codex_cloud_wrapper_protocol::RequestRunParams;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::TerminalSignal;
use codex_cloud_wrapper_protocol::ThreadItemRecord;
use codex_cloud_wrapper_protocol::ThreadItemsListParams;
use pretty_assertions::assert_eq;
use sqlx::MySqlPool;
use uuid::Uuid;

use crate::CloudRequestService;
use crate::CloudStateError;
use crate::CloudStateStore;
use crate::CreateRequestParams;
use crate::EventAppendParams;
use crate::LeaseAcquireOutcome;
use crate::LeaseAppendParams;
use crate::MysqlCloudStateSchema;
use crate::MysqlCloudStateStore;
use crate::TerminalStatusUpdate;

#[test]
fn mysql_schema_declares_request_lease_append_and_event_tables() {
    let statements = MysqlCloudStateSchema::create_table_statements().join("\n");

    assert!(statements.contains("CREATE TABLE IF NOT EXISTS cloud_requests"));
    assert!(statements.contains("UNIQUE KEY cloud_requests_idempotency_key"));
    assert!(statements.contains("CREATE TABLE IF NOT EXISTS cloud_thread_writer_leases"));
    assert!(statements.contains("UNIQUE KEY cloud_thread_writer_leases_thread_id"));
    assert!(statements.contains("CREATE TABLE IF NOT EXISTS cloud_fencing_tokens"));
    assert!(statements.contains("CREATE TABLE IF NOT EXISTS cloud_append_results"));
    assert!(statements.contains("request_id VARCHAR(128) NULL"));
    assert!(statements.contains("payload_refs_json JSON NULL"));
    assert!(statements.contains("UNIQUE KEY cloud_append_results_idempotency_key"));
    assert!(statements.contains("CREATE TABLE IF NOT EXISTS cloud_thread_items"));
    assert!(statements.contains("CREATE TABLE IF NOT EXISTS cloud_request_events"));
    assert!(statements.contains("UNIQUE KEY cloud_request_events_sequence"));
}

#[test]
fn mysql_store_implements_cloud_state_store_contract() {
    fn assert_cloud_state_store<T: CloudStateStore + Send>() {}

    assert_cloud_state_store::<MysqlCloudStateStore>();
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_persists_request_lifecycle_when_database_url_is_configured() {
    let database_url = std::env::var("CODEX_CLOUD_STATE_MYSQL_URL")
        .expect("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test");
    let pool = MySqlPool::connect(database_url.as_str())
        .await
        .expect("connect to MySQL test database");
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    clear_mysql_state(&pool).await;

    let mut service = CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()));
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "sha256:input-1".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request/run should persist request");
    let read = service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: run.request.request_id.clone(),
            },
        )
        .await
        .expect("request/read should load persisted request");
    let cancel = service
        .cancel(
            "caller-1",
            RequestCancelParams {
                request_id: run.request.request_id.clone(),
                reason: Some("test".to_string()),
            },
        )
        .await
        .expect("request/cancel should persist terminal status");
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id.clone(),
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .expect("request/events/list should load persisted events");

    assert_eq!(run.request.status, RequestStatus::Running);
    assert_eq!(read.request, run.request);
    assert_eq!(cancel.request.status, RequestStatus::Cancelled);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running", "request/cancelled"]
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_service_request_operations_are_scoped_to_the_request_caller_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let mut service = CloudRequestService::new(MysqlCloudStateStore::new(pool));
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some(format!("thread-{unique}")),
                create_thread: None,
                input: format!("sha256:input-{unique}"),
                idempotency_key: format!("idem-{unique}"),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request/run should persist request");
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    let read_error = service
        .read(
            "caller-2",
            RequestReadParams {
                request_id: run.request.request_id.clone(),
            },
        )
        .await
        .expect_err("different caller should not read request");
    let events_error = service
        .events_list(
            "caller-2",
            RequestEventsListParams {
                request_id: run.request.request_id.clone(),
                cursor: None,
                limit: None,
            },
        )
        .await
        .expect_err("different caller should not read events");
    let append_error = service
        .append_thread_items_with_lease(
            "caller-2",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id.clone(),
                request_id: run.request.request_id.clone(),
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: format!("append-{unique}"),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: format!("object://items/{unique}/1"),
                }],
            },
        )
        .await
        .expect_err("different caller should not append thread items");
    let thread_items_error = service
        .thread_items_list(
            "caller-2",
            ThreadItemsListParams {
                thread_id: run.thread_id,
                cursor: None,
                limit: None,
            },
        )
        .await
        .expect_err("different caller should not list thread items");
    let cancel_error = service
        .cancel(
            "caller-2",
            RequestCancelParams {
                request_id: run.request.request_id.clone(),
                reason: Some("wrong caller".to_string()),
            },
        )
        .await
        .expect_err("different caller should not cancel request");
    let terminal_error = service
        .terminal_signal(
            "caller-2",
            &run.request.request_id,
            TerminalSignal::Completed,
            None,
        )
        .await
        .expect_err("different caller should not mark terminal request");
    let final_read = service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: run.request.request_id,
            },
        )
        .await
        .expect("original caller should still read request");

    assert_eq!(read_error, CloudStateError::RequestNotFound);
    assert_eq!(events_error, CloudStateError::RequestNotFound);
    assert_eq!(append_error, CloudStateError::RequestNotFound);
    assert_eq!(thread_items_error, CloudStateError::RequestNotFound);
    assert_eq!(cancel_error, CloudStateError::RequestNotFound);
    assert_eq!(terminal_error, CloudStateError::RequestNotFound);
    assert_eq!(final_read.request.status, RequestStatus::Running);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_serializes_concurrent_lease_acquire_when_database_url_is_configured() {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let first = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let second = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-2",
    )
    .await;

    let first_request_id = first.request_id.clone();
    let second_request_id = second.request_id.clone();
    let first_acquire = async {
        let mut store = MysqlCloudStateStore::new(pool.clone());
        store
            .acquire_thread_writer_lease(&first_request_id, "owner-1")
            .await
    };
    let second_acquire = async {
        let mut store = MysqlCloudStateStore::new(pool.clone());
        store
            .acquire_thread_writer_lease(&second_request_id, "owner-2")
            .await
    };
    let (first_outcome, second_outcome) = tokio::join!(first_acquire, second_acquire);
    let outcomes = [
        first_outcome.expect("first acquire should not fail"),
        second_outcome.expect("second acquire should not fail"),
    ];

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, LeaseAcquireOutcome::Acquired { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, LeaseAcquireOutcome::Queued))
            .count(),
        1
    );

    let first = MysqlCloudStateStore::new(pool.clone())
        .read_request(&first.request_id)
        .await
        .expect("read first request");
    let second = MysqlCloudStateStore::new(pool)
        .read_request(&second.request_id)
        .await
        .expect("read second request");
    assert_eq!(
        [first.status, second.status]
            .into_iter()
            .filter(|status| *status == RequestStatus::Running)
            .count(),
        1
    );
    assert_eq!(
        [first.status, second.status]
            .into_iter()
            .filter(|status| *status == RequestStatus::Queued)
            .count(),
        1
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_terminal_queued_request_keeps_running_request_lease_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let running = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let queued = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-2",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&running.request_id, "owner-1")
        .await
        .expect("acquire lease")
    else {
        panic!("expected request to acquire lease");
    };

    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .mark_request_terminal(&queued.request_id, RequestStatus::Cancelled)
            .await
            .expect("cancel queued request"),
        TerminalStatusUpdate::Changed
    );
    let append = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: running.request_id.clone(),
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .await
        .expect("running request lease should remain valid");

    assert_eq!(append.first_append_sequence, 1);
    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .read_request(&running.request_id)
            .await
            .expect("read running request")
            .status,
        RequestStatus::Running
    );
    assert_eq!(
        MysqlCloudStateStore::new(pool)
            .read_request(&queued.request_id)
            .await
            .expect("read queued request")
            .status,
        RequestStatus::Cancelled
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_append_from_queued_request_without_held_lease_does_not_mark_it_lease_lost_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let running = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-running",
    )
    .await;
    let queued = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-queued",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&running.request_id, "owner-1")
        .await
        .expect("acquire running lease")
    else {
        panic!("expected running request to acquire lease");
    };

    let error = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: queued.request_id.clone(),
            lease_id: "lease-missing".to_string(),
            writer_owner_token: "owner-2".to_string(),
            fencing_token: 99,
            append_idempotency_key: "append-queued".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/queued".to_string()],
        })
        .await
        .expect_err("queued request without held lease should not append");
    let append = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: running.request_id.clone(),
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-running".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/running".to_string()],
        })
        .await
        .expect("running request should keep its lease");

    assert_eq!(error, CloudStateError::LeaseLost);
    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .read_request(&queued.request_id)
            .await
            .expect("read queued request")
            .status,
        RequestStatus::Queued
    );
    assert_eq!(
        MysqlCloudStateStore::new(pool)
            .read_request(&running.request_id)
            .await
            .expect("read running request")
            .status,
        RequestStatus::Running
    );
    assert_eq!(append.first_append_sequence, 1);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_serializes_concurrent_appends_with_same_lease_when_database_url_is_configured()
{
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease")
    else {
        panic!("expected request to acquire lease");
    };

    let request_id = request.request_id.clone();
    let first_append = append_with_key(
        MysqlCloudStateStore::new(pool.clone()),
        request_id.clone(),
        lease_id.clone(),
        fencing_token,
        "append-1",
    );
    let second_append = append_with_key(
        MysqlCloudStateStore::new(pool),
        request_id,
        lease_id,
        fencing_token,
        "append-2",
    );
    let (first, second) = tokio::join!(first_append, second_append);
    let mut ranges = vec![
        first
            .expect("first append should succeed")
            .first_append_sequence,
        second
            .expect("second append should succeed")
            .first_append_sequence,
    ];
    ranges.sort_unstable();

    assert_eq!(ranges, vec![1, 2]);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_rejects_stale_expected_thread_version_when_database_url_is_configured() {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease")
    else {
        panic!("expected request to acquire lease");
    };

    MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id.clone(),
            lease_id: lease_id.clone(),
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .await
        .expect("first append should succeed");
    let error = MysqlCloudStateStore::new(pool)
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id,
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-2".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/2".to_string()],
        })
        .await
        .expect_err("stale expected thread version should be rejected");

    assert_eq!(error, CloudStateError::ThreadVersionConflict);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_rejects_empty_append_when_database_url_is_configured() {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease")
    else {
        panic!("expected request to acquire lease");
    };

    let error = MysqlCloudStateStore::new(pool)
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id,
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: Vec::new(),
        })
        .await
        .expect_err("empty append should be rejected");

    assert_eq!(error, CloudStateError::EmptyAppend);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_persists_thread_items_when_database_url_is_configured() {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease")
    else {
        panic!("expected request to acquire lease");
    };
    let replay_lease_id = lease_id.clone();
    let replay_fencing_token = fencing_token;

    let params = LeaseAppendParams {
        request_id: request.request_id.clone(),
        lease_id,
        writer_owner_token: "owner-1".to_string(),
        fencing_token,
        append_idempotency_key: "append-1".to_string(),
        expected_thread_version: Some(0),
        payload_refs: vec![
            "object://items/1".to_string(),
            "object://items/2".to_string(),
        ],
    };
    let first = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(params.clone())
        .await
        .expect("append should succeed");
    let replay = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(params.clone())
        .await
        .expect("append replay should deduplicate");
    let mut conflicting_replay = params.clone();
    conflicting_replay.payload_refs = vec!["object://items/different".to_string()];
    let error = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(conflicting_replay)
        .await
        .expect_err("append replay with different payload refs should fail");
    let mut replay_with_wrong_fencing_token = LeaseAppendParams {
        request_id: request.request_id.clone(),
        lease_id: replay_lease_id.clone(),
        writer_owner_token: "owner-1".to_string(),
        fencing_token: replay_fencing_token,
        append_idempotency_key: "append-1".to_string(),
        expected_thread_version: Some(0),
        payload_refs: vec![
            "object://items/1".to_string(),
            "object://items/2".to_string(),
        ],
    };
    replay_with_wrong_fencing_token.fencing_token = 99;
    let lease_error = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(replay_with_wrong_fencing_token)
        .await
        .expect_err("append replay with wrong fencing token should fail before dedupe");
    let (items, next_cursor) = MysqlCloudStateStore::new(pool)
        .list_thread_items(&request.thread_id, /*cursor*/ None, /*limit*/ 10)
        .await
        .expect("list thread items");

    assert_eq!(first.thread_version, 2);
    assert_eq!(replay.deduplicated, true);
    assert_eq!(error, CloudStateError::IdempotencyConflict);
    assert_eq!(lease_error, CloudStateError::LeaseLost);
    assert_eq!(
        items,
        vec![
            ThreadItemRecord {
                thread_id: request.thread_id.clone(),
                sequence: 1,
                payload_ref: "object://items/1".to_string(),
            },
            ThreadItemRecord {
                thread_id: request.thread_id,
                sequence: 2,
                payload_ref: "object://items/2".to_string(),
            },
        ]
    );
    assert_eq!(next_cursor, None);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_scopes_append_idempotency_replay_to_original_request_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let first_request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let second_request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-2",
    )
    .await;
    let LeaseAcquireOutcome::Acquired {
        lease_id: first_lease_id,
        fencing_token: first_fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&first_request.request_id, "owner-1")
        .await
        .expect("acquire first lease")
    else {
        panic!("expected first request to acquire lease");
    };
    MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: first_request.request_id.clone(),
            lease_id: first_lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token: first_fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .await
        .expect("first append should succeed");
    MysqlCloudStateStore::new(pool.clone())
        .mark_request_terminal(&first_request.request_id, RequestStatus::Completed)
        .await
        .expect("complete first request");
    let LeaseAcquireOutcome::Acquired {
        lease_id: second_lease_id,
        fencing_token: second_fencing_token,
    } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&second_request.request_id, "owner-2")
        .await
        .expect("acquire second lease")
    else {
        panic!("expected second request to acquire lease");
    };

    let error = MysqlCloudStateStore::new(pool)
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: second_request.request_id,
            lease_id: second_lease_id,
            writer_owner_token: "owner-2".to_string(),
            fencing_token: second_fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(1),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .await
        .expect_err("append replay from a different request should fail");

    assert_eq!(error, CloudStateError::IdempotencyConflict);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_serializes_concurrent_request_events_when_database_url_is_configured() {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;

    let request_id = request.request_id.clone();
    let first_append = async {
        MysqlCloudStateStore::new(pool.clone())
            .append_request_event(EventAppendParams {
                request_id: request_id.clone(),
                event_type: "test/first".to_string(),
                payload_inline: "{}".to_string(),
            })
            .await
    };
    let second_append = async {
        MysqlCloudStateStore::new(pool.clone())
            .append_request_event(EventAppendParams {
                request_id: request_id.clone(),
                event_type: "test/second".to_string(),
                payload_inline: "{}".to_string(),
            })
            .await
    };
    let (first, second) = tokio::join!(first_append, second_append);
    let first = first.expect("first event append should succeed");
    let second = second.expect("second event append should succeed");
    let mut sequences = vec![first.sequence, second.sequence];
    sequences.sort_unstable();
    let final_request = MysqlCloudStateStore::new(pool.clone())
        .read_request(&request_id)
        .await
        .expect("read final request");
    let events = MysqlCloudStateStore::new(pool)
        .list_request_events(&request_id, None, 10)
        .await
        .expect("list request events");

    assert_eq!(sequences, vec![1, 2]);
    assert_eq!(final_request.latest_event_cursor, 2);
    assert_eq!(events.data.len(), 2);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_rejects_invalid_request_event_payload_without_advancing_cursor_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;

    let error = MysqlCloudStateStore::new(pool.clone())
        .append_request_event(EventAppendParams {
            request_id: request.request_id.clone(),
            event_type: "request/invalid".to_string(),
            payload_inline: "not-json".to_string(),
        })
        .await
        .expect_err("invalid event payload should fail");

    assert_eq!(error, CloudStateError::InvalidEventPayload);
    assert_eq!(
        MysqlCloudStateStore::new(pool)
            .read_request(&request.request_id)
            .await
            .expect("read request")
            .latest_event_cursor,
        0
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_marks_stale_lease_append_as_lease_lost_when_database_url_is_configured() {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let queued = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-2",
    )
    .await;
    let LeaseAcquireOutcome::Acquired { lease_id, .. } = MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease")
    else {
        panic!("expected request to acquire lease");
    };

    let error = MysqlCloudStateStore::new(pool.clone())
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id.clone(),
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token: 999,
            append_idempotency_key: "append-stale".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://item/stale".to_string()],
        })
        .await
        .expect_err("stale fencing token should be rejected");

    assert_eq!(error, CloudStateError::LeaseLost);
    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .read_request(&request.request_id)
            .await
            .expect("read request")
            .status,
        RequestStatus::LeaseLost
    );
    assert!(matches!(
        MysqlCloudStateStore::new(pool)
            .acquire_thread_writer_lease(&queued.request_id, "owner-2")
            .await
            .expect("queued request should acquire released lease"),
        LeaseAcquireOutcome::Acquired { .. }
    ));
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_owner_timeout_releases_lease_for_next_queued_request_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let first = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;
    let second = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-2",
    )
    .await;

    assert!(matches!(
        MysqlCloudStateStore::new(pool.clone())
            .acquire_thread_writer_lease(&first.request_id, "owner-1")
            .await
            .expect("first acquire lease"),
        LeaseAcquireOutcome::Acquired { .. }
    ));
    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .acquire_thread_writer_lease(&second.request_id, "owner-2")
            .await
            .expect("second should queue"),
        LeaseAcquireOutcome::Queued
    );

    MysqlCloudStateStore::new(pool.clone())
        .mark_request_terminal(&first.request_id, RequestStatus::OwnerTimedOut)
        .await
        .expect("mark owner timed out");
    assert!(matches!(
        MysqlCloudStateStore::new(pool.clone())
            .acquire_thread_writer_lease(&second.request_id, "owner-2")
            .await
            .expect("second acquire lease"),
        LeaseAcquireOutcome::Acquired { .. }
    ));

    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .read_request(&first.request_id)
            .await
            .expect("read first request")
            .status,
        RequestStatus::OwnerTimedOut
    );
    assert_eq!(
        MysqlCloudStateStore::new(pool)
            .read_request(&second.request_id)
            .await
            .expect("read second request")
            .status,
        RequestStatus::Running
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_service_replays_queued_request_after_owner_timeout_when_database_url_is_configured()
{
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let thread_id = format!("thread-{unique}");
    let first_params = RequestRunParams {
        thread_id: Some(thread_id.clone()),
        create_thread: None,
        input: format!("sha256:first-{unique}"),
        idempotency_key: format!("idem-first-{unique}"),
        cwd: None,
        client_info: None,
    };
    let second_params = RequestRunParams {
        thread_id: Some(thread_id),
        create_thread: None,
        input: format!("sha256:second-{unique}"),
        idempotency_key: format!("idem-second-{unique}"),
        cwd: None,
        client_info: None,
    };
    let mut first_service = CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()));
    let first = first_service
        .run("caller-1", first_params)
        .await
        .expect("first request should run");
    let queued = first_service
        .run("caller-2", second_params.clone())
        .await
        .expect("second request should queue");

    let mut timeout_service = CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()));
    let timed_out = timeout_service
        .terminal_signal(
            "caller-1",
            &first.request.request_id,
            TerminalSignal::OwnerLeaseTimedOut,
            None,
        )
        .await
        .expect("owner timeout should persist");
    let mut replay_service = CloudRequestService::new(MysqlCloudStateStore::new(pool));
    let replay = replay_service
        .run("caller-2", second_params)
        .await
        .expect("queued request replay should acquire released lease");

    assert_eq!(first.request.status, RequestStatus::Running);
    assert_eq!(queued.request.status, RequestStatus::Queued);
    assert_eq!(timed_out.request.status, RequestStatus::OwnerTimedOut);
    assert_eq!(replay.request.status, RequestStatus::Running);
    assert_eq!(replay.request.request_id, queued.request.request_id);
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_service_concurrent_idempotent_run_appends_initial_events_once_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let params = RequestRunParams {
        thread_id: Some(format!("thread-{unique}")),
        create_thread: None,
        input: format!("sha256:input-{unique}"),
        idempotency_key: format!("idem-{unique}"),
        cwd: None,
        client_info: None,
    };
    let first_params = params.clone();
    let second_params = params;

    let first_run = async {
        CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()))
            .run("caller-1", first_params)
            .await
    };
    let second_run = async {
        CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()))
            .run("caller-1", second_params)
            .await
    };
    let (first, second) = tokio::join!(first_run, second_run);
    let first = first.expect("first request/run should succeed");
    let second = second.expect("second request/run should succeed");
    let events = CloudRequestService::new(MysqlCloudStateStore::new(pool))
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: first.request.request_id.clone(),
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .expect("request/events/list should load");

    assert_eq!(second.request.request_id, first.request.request_id);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_service_concurrent_idempotent_run_conflict_keeps_single_initialized_request_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let first_params = RequestRunParams {
        thread_id: Some(format!("thread-{unique}")),
        create_thread: None,
        input: format!("sha256:first-{unique}"),
        idempotency_key: format!("idem-{unique}"),
        cwd: None,
        client_info: None,
    };
    let second_params = RequestRunParams {
        input: format!("sha256:second-{unique}"),
        ..first_params.clone()
    };

    let first_run = async {
        CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()))
            .run("caller-1", first_params)
            .await
    };
    let second_run = async {
        CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()))
            .run("caller-1", second_params)
            .await
    };
    let (first, second) = tokio::join!(first_run, second_run);
    let outcomes = [first, second];
    let successful = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok())
        .collect::<Vec<_>>();
    let conflicts = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Err(CloudStateError::IdempotencyConflict)))
        .count();
    let events = CloudRequestService::new(MysqlCloudStateStore::new(pool))
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: successful
                    .first()
                    .expect("one request/run should succeed")
                    .request
                    .request_id
                    .clone(),
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .expect("request/events/list should load");

    assert_eq!(successful.len(), 1);
    assert_eq!(conflicts, 1);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_service_concurrent_terminal_signals_append_only_one_terminal_event_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let thread_id = format!("thread-{unique}");
    let mut service = CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()));
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some(thread_id),
                create_thread: None,
                input: format!("sha256:input-{unique}"),
                idempotency_key: format!("idem-{unique}"),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request should run");

    let request_id = run.request.request_id.clone();
    let first_terminal = async {
        CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()))
            .terminal_signal(
                "caller-1",
                &request_id,
                TerminalSignal::Completed,
                Some(r#"{"worker":"first"}"#.to_string()),
            )
            .await
    };
    let second_terminal = async {
        CloudRequestService::new(MysqlCloudStateStore::new(pool.clone()))
            .terminal_signal(
                "caller-1",
                &request_id,
                TerminalSignal::Failed,
                Some(r#"{"worker":"second"}"#.to_string()),
            )
            .await
    };
    let (first, second) = tokio::join!(first_terminal, second_terminal);
    let first = first.expect("first terminal signal should succeed");
    let second = second.expect("second terminal signal should succeed");
    let final_service = CloudRequestService::new(MysqlCloudStateStore::new(pool));
    let final_request = final_service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: request_id.clone(),
            },
        )
        .await
        .expect("final request should load");
    let events = final_service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: request_id.clone(),
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .expect("request events should load");
    let event_types = events
        .data
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();

    assert_eq!(first.request.status, second.request.status);
    assert!(matches!(
        first.request.status,
        RequestStatus::Completed | RequestStatus::Failed
    ));
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| matches!(**event_type, "request/completed" | "request/failed"))
            .count(),
        1
    );
    assert_eq!(event_types.len(), 3);
    assert_eq!(event_types[0], "request/queued");
    assert_eq!(event_types[1], "request/running");
    assert_eq!(
        events.data.last().map(|event| event.sequence),
        Some(final_request.latest_event_cursor)
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_terminal_request_cannot_reacquire_lease_and_terminal_status_is_stable_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;

    MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease");
    let first_update = MysqlCloudStateStore::new(pool.clone())
        .mark_request_terminal(&request.request_id, RequestStatus::OwnerTimedOut)
        .await
        .expect("mark owner timed out");
    let second_update = MysqlCloudStateStore::new(pool.clone())
        .mark_request_terminal(&request.request_id, RequestStatus::Failed)
        .await
        .expect("terminal status should not be overwritten");

    assert_eq!(first_update, TerminalStatusUpdate::Changed);
    assert_eq!(second_update, TerminalStatusUpdate::Unchanged);
    assert_eq!(
        MysqlCloudStateStore::new(pool.clone())
            .acquire_thread_writer_lease(&request.request_id, "owner-1")
            .await
            .expect_err("terminal request should not reacquire lease"),
        CloudStateError::RequestNotQueued
    );
    assert_eq!(
        MysqlCloudStateStore::new(pool)
            .read_request(&request.request_id)
            .await
            .expect("read request")
            .status,
        RequestStatus::OwnerTimedOut
    );
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn mysql_store_terminal_request_with_event_updates_status_cursor_and_event_in_one_store_call_when_database_url_is_configured()
 {
    let pool = mysql_pool().await;
    let store = MysqlCloudStateStore::new(pool.clone());
    store
        .create_schema()
        .await
        .expect("create cloud state schema");
    let unique = unique_key();
    let request = create_request(
        &mut MysqlCloudStateStore::new(pool.clone()),
        &unique,
        "idem-1",
    )
    .await;

    MysqlCloudStateStore::new(pool.clone())
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .await
        .expect("acquire lease");
    let update = MysqlCloudStateStore::new(pool.clone())
        .mark_request_terminal_with_event(
            EventAppendParams {
                request_id: request.request_id.clone(),
                event_type: "request/completed".to_string(),
                payload_inline: "{\"ok\":true}".to_string(),
            },
            RequestStatus::Completed,
        )
        .await
        .expect("mark terminal and append event");
    let persisted = MysqlCloudStateStore::new(pool.clone())
        .read_request(&request.request_id)
        .await
        .expect("read request");
    let events = MysqlCloudStateStore::new(pool.clone())
        .list_request_events(&request.request_id, /*cursor*/ None, /*limit*/ 10)
        .await
        .expect("list request events");

    assert_eq!(update, TerminalStatusUpdate::Changed);
    assert_eq!(persisted.status, RequestStatus::Completed);
    assert_eq!(persisted.latest_event_cursor, 1);
    assert_eq!(
        MysqlCloudStateStore::new(pool)
            .read_thread_writer_lease(&request.request_id)
            .await
            .expect("read writer lease"),
        None
    );
    assert_eq!(
        events.data,
        vec![crate::EventRecord {
            request_id: request.request_id,
            sequence: 1,
            event_type: "request/completed".to_string(),
            payload_inline: "{\"ok\": true}".to_string(),
        }]
    );
}

async fn clear_mysql_state(pool: &MySqlPool) {
    sqlx::query("DELETE FROM cloud_request_events")
        .execute(pool)
        .await
        .expect("clear cloud_request_events");
    sqlx::query("DELETE FROM cloud_thread_items")
        .execute(pool)
        .await
        .expect("clear cloud_thread_items");
    sqlx::query("DELETE FROM cloud_append_results")
        .execute(pool)
        .await
        .expect("clear cloud_append_results");
    sqlx::query("DELETE FROM cloud_thread_writer_leases")
        .execute(pool)
        .await
        .expect("clear cloud_thread_writer_leases");
    sqlx::query("DELETE FROM cloud_fencing_tokens")
        .execute(pool)
        .await
        .expect("clear cloud_fencing_tokens");
    sqlx::query("DELETE FROM cloud_requests")
        .execute(pool)
        .await
        .expect("clear cloud_requests");
}

async fn mysql_pool() -> MySqlPool {
    let database_url = std::env::var("CODEX_CLOUD_STATE_MYSQL_URL")
        .expect("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test");
    MySqlPool::connect(database_url.as_str())
        .await
        .expect("connect to MySQL test database")
}

fn unique_key() -> String {
    Uuid::now_v7().to_string()
}

async fn create_request(
    store: &mut MysqlCloudStateStore,
    unique: &str,
    idempotency_key: &str,
) -> crate::RequestRecord {
    store
        .create_request(CreateRequestParams {
            caller_id: format!("caller-{unique}"),
            thread_id: format!("thread-{unique}"),
            idempotency_key: idempotency_key.to_string(),
            input_hash: format!("hash-{idempotency_key}"),
        })
        .await
        .expect("create request")
}

async fn append_with_key(
    mut store: MysqlCloudStateStore,
    request_id: String,
    lease_id: String,
    fencing_token: u64,
    append_idempotency_key: &str,
) -> Result<codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseResponse, CloudStateError> {
    store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id,
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: append_idempotency_key.to_string(),
            expected_thread_version: None,
            payload_refs: vec![format!("object://item/{append_idempotency_key}")],
        })
        .await
}
