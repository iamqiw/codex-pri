use codex_cloud_wrapper_protocol::AppendThreadItem;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseParams;
use codex_cloud_wrapper_protocol::RequestCancelParams;
use codex_cloud_wrapper_protocol::RequestEventsListParams;
use codex_cloud_wrapper_protocol::RequestReadParams;
use codex_cloud_wrapper_protocol::RequestRunParams;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::RequestTerminalParams;
use codex_cloud_wrapper_protocol::TerminalSignal;
use codex_cloud_wrapper_protocol::ThreadItemsListParams;
use pretty_assertions::assert_eq;
use serde_json::Value;

use crate::CloudRequestService;
use crate::CloudStateError;
use crate::CreateRequestParams;
use crate::EventAppendParams;
use crate::InMemoryCloudStateStore;

#[tokio::test]
async fn request_service_can_run_against_an_injected_store() {
    let mut service = CloudRequestService::new(InMemoryCloudStateStore::default());

    let response = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(response.request.status, RequestStatus::Running);
}

#[tokio::test]
async fn request_run_is_idempotent_and_events_can_be_read_by_cursor() {
    let mut service = CloudRequestService::default();
    let params = RequestRunParams {
        thread_id: Some("thread-1".to_string()),
        create_thread: None,
        input: "hello".to_string(),
        idempotency_key: "idem-1".to_string(),
        cwd: Some("/workspace".to_string()),
        client_info: Some("integration-test".to_string()),
    };

    let first = service.run("caller-1", params.clone()).await.unwrap();
    let second = service.run("caller-1", params).await.unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: first.request.request_id.clone(),
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();
    let read = service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: first.request.request_id.clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(second, first);
    assert_eq!(first.request.status, RequestStatus::Running);
    assert_eq!(first.thread_id, "thread-1");
    assert_eq!(first.event_cursor, 2);
    assert_eq!(read.request, first.request);
    assert_eq!(read.latest_event_cursor, 2);
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
async fn request_run_returns_writer_lease_that_can_append_thread_items() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should include writer lease");

    let append = service
        .append_thread_items_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id,
                request_id: run.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: "object://items/1".to_string(),
                }],
            },
        )
        .await
        .unwrap();

    assert_eq!(append.thread_version, 1);
    assert_eq!(append.first_append_sequence, 1);
    assert_eq!(append.last_append_sequence, 1);
    assert!(!append.deduplicated);
}

#[tokio::test]
async fn request_operations_are_scoped_to_the_request_caller() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should include writer lease");

    let read_error = service
        .read(
            "caller-2",
            RequestReadParams {
                request_id: run.request.request_id.clone(),
            },
        )
        .await
        .unwrap_err();
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
        .unwrap_err();
    let append_error = service
        .append_thread_items_with_lease(
            "caller-2",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id.clone(),
                request_id: run.request.request_id.clone(),
                lease_id: writer_lease.lease_id.clone(),
                writer_owner_token: writer_lease.writer_owner_token.clone(),
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: "object://items/1".to_string(),
                }],
            },
        )
        .await
        .unwrap_err();
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
        .unwrap_err();
    let cancel_error = service
        .cancel(
            "caller-2",
            RequestCancelParams {
                request_id: run.request.request_id.clone(),
                reason: Some("wrong caller".to_string()),
            },
        )
        .await
        .unwrap_err();
    let terminal_error = service
        .terminal_signal(
            "caller-2",
            &run.request.request_id,
            TerminalSignal::Completed,
            None,
        )
        .await
        .unwrap_err();
    let final_read = service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: run.request.request_id,
            },
        )
        .await
        .unwrap();

    assert_eq!(read_error, CloudStateError::RequestNotFound);
    assert_eq!(events_error, CloudStateError::RequestNotFound);
    assert_eq!(append_error, CloudStateError::RequestNotFound);
    assert_eq!(thread_items_error, CloudStateError::RequestNotFound);
    assert_eq!(cancel_error, CloudStateError::RequestNotFound);
    assert_eq!(terminal_error, CloudStateError::RequestNotFound);
    assert_eq!(final_read.request.status, RequestStatus::Running);
}

#[tokio::test]
async fn append_thread_items_rejects_stale_expected_thread_version() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should include writer lease");

    service
        .append_thread_items_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id.clone(),
                request_id: run.request.request_id.clone(),
                lease_id: writer_lease.lease_id.clone(),
                writer_owner_token: writer_lease.writer_owner_token.clone(),
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: "object://items/1".to_string(),
                }],
            },
        )
        .await
        .unwrap();
    let error = service
        .append_thread_items_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id,
                request_id: run.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-2".to_string(),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: "object://items/2".to_string(),
                }],
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error, CloudStateError::ThreadVersionConflict);
}

#[tokio::test]
async fn append_thread_items_rejects_empty_items() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should include writer lease");

    let error = service
        .append_thread_items_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id,
                request_id: run.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: Vec::new(),
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error, CloudStateError::EmptyAppend);
}

#[tokio::test]
async fn thread_items_list_caps_explicit_large_limit() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should include writer lease");
    let items = (1..=101)
        .map(|sequence| AppendThreadItem {
            payload_ref: format!("object://items/{sequence}"),
        })
        .collect::<Vec<_>>();

    service
        .append_thread_items_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id.clone(),
                request_id: run.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items,
            },
        )
        .await
        .unwrap();
    let page = service
        .thread_items_list(
            "caller-1",
            ThreadItemsListParams {
                thread_id: run.thread_id,
                cursor: None,
                limit: Some(1_000),
            },
        )
        .await
        .unwrap();

    assert_eq!(page.data.len(), 100);
    assert_eq!(page.next_cursor, Some(100));
}

#[tokio::test]
async fn thread_items_list_clamps_zero_limit_to_one() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let writer_lease = run
        .writer_lease
        .clone()
        .expect("running request should include writer lease");

    service
        .append_thread_items_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run.thread_id.clone(),
                request_id: run.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: vec![
                    AppendThreadItem {
                        payload_ref: "object://items/1".to_string(),
                    },
                    AppendThreadItem {
                        payload_ref: "object://items/2".to_string(),
                    },
                ],
            },
        )
        .await
        .unwrap();
    let page = service
        .thread_items_list(
            "caller-1",
            ThreadItemsListParams {
                thread_id: run.thread_id,
                cursor: None,
                limit: Some(0),
            },
        )
        .await
        .unwrap();

    assert_eq!(page.data.len(), 1);
    assert_eq!(page.next_cursor, Some(1));
}

#[tokio::test]
async fn request_events_list_caps_explicit_large_limit() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hello".to_string(),
        })
        .unwrap();
    for sequence in 1..=101 {
        store
            .append_request_event(EventAppendParams {
                request_id: request.request_id.clone(),
                event_type: "request/progress".to_string(),
                payload_inline: format!(r#"{{"sequence":{sequence}}}"#),
            })
            .unwrap();
    }
    let service = CloudRequestService::new(store);

    let page = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: request.request_id,
                cursor: None,
                limit: Some(1_000),
            },
        )
        .await
        .unwrap();

    assert_eq!(page.data.len(), 100);
    assert_eq!(page.next_cursor, Some(100));
}

#[tokio::test]
async fn request_events_list_clamps_zero_limit_to_one() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hello".to_string(),
        })
        .unwrap();
    for sequence in 1..=2 {
        store
            .append_request_event(EventAppendParams {
                request_id: request.request_id.clone(),
                event_type: "request/progress".to_string(),
                payload_inline: format!(r#"{{"sequence":{sequence}}}"#),
            })
            .unwrap();
    }
    let service = CloudRequestService::new(store);

    let page = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: request.request_id,
                cursor: None,
                limit: Some(0),
            },
        )
        .await
        .unwrap();

    assert_eq!(page.data.len(), 1);
    assert_eq!(page.next_cursor, Some(1));
}

#[tokio::test]
async fn request_cancel_marks_request_cancelled_and_records_event() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    let cancel = service
        .cancel(
            "caller-1",
            RequestCancelParams {
                request_id: run.request.request_id.clone(),
                reason: Some("user_cancelled".to_string()),
            },
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: Some(2),
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(cancel.request.status, RequestStatus::Cancelled);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/cancelled"]
    );
}

#[tokio::test]
async fn request_cancel_reason_payload_is_valid_json() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();
    let reason = "operator said \"stop\" \\ retry later";

    service
        .cancel(
            "caller-1",
            RequestCancelParams {
                request_id: run.request.request_id.clone(),
                reason: Some(reason.to_string()),
            },
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: Some(2),
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(
        serde_json::from_str::<Value>(&events.data[0].payload_inline).unwrap(),
        serde_json::json!({ "reason": reason })
    );
}

#[tokio::test]
async fn queued_request_can_resume_after_owner_terminal_signal_releases_lease() {
    let mut service = CloudRequestService::default();
    let first_params = RequestRunParams {
        thread_id: Some("thread-1".to_string()),
        create_thread: None,
        input: "first".to_string(),
        idempotency_key: "idem-1".to_string(),
        cwd: None,
        client_info: None,
    };
    let second_params = RequestRunParams {
        thread_id: Some("thread-1".to_string()),
        create_thread: None,
        input: "second".to_string(),
        idempotency_key: "idem-2".to_string(),
        cwd: None,
        client_info: None,
    };

    let first = service.run("caller-1", first_params).await.unwrap();
    let queued = service
        .run("caller-2", second_params.clone())
        .await
        .unwrap();
    let timed_out = service
        .terminal_signal(
            "caller-1",
            &first.request.request_id,
            TerminalSignal::OwnerLeaseTimedOut,
            None,
        )
        .await
        .unwrap();
    let resumed = service.run("caller-2", second_params).await.unwrap();
    let resumed_events = service
        .events_list(
            "caller-2",
            RequestEventsListParams {
                request_id: resumed.request.request_id.clone(),
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(queued.request.status, RequestStatus::Queued);
    assert_eq!(timed_out.request.status, RequestStatus::OwnerTimedOut);
    assert_eq!(resumed.request.status, RequestStatus::Running);
    assert_eq!(
        resumed_events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );
}

#[tokio::test]
async fn completed_request_is_not_overwritten_by_later_terminal_signal() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    let completed = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::Completed,
            None,
        )
        .await
        .unwrap();
    let after_timeout = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::OwnerLeaseTimedOut,
            None,
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(completed.request.status, RequestStatus::Completed);
    assert_eq!(after_timeout.request, completed.request);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running", "request/completed"]
    );
}

#[tokio::test]
async fn terminal_request_is_not_overwritten_by_later_terminal_signal() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    let cancelled = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::CancelConfirmed,
            None,
        )
        .await
        .unwrap();
    let after_failed = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::Failed,
            None,
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(cancelled.request.status, RequestStatus::Cancelled);
    assert_eq!(after_failed.request, cancelled.request);
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
async fn completed_request_ignores_invalid_payload_from_later_terminal_signal() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    let completed = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::Completed,
            None,
        )
        .await
        .unwrap();
    let after_failed = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::Failed,
            Some("not-json".to_string()),
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(after_failed.request, completed.request);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running", "request/completed"]
    );
}

#[tokio::test]
async fn terminal_rejects_invalid_json_payload_without_changing_request() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    let error = service
        .terminal(
            "caller-1",
            RequestTerminalParams {
                request_id: run.request.request_id.clone(),
                signal: TerminalSignal::Failed,
                payload_inline: Some("not-json".to_string()),
            },
        )
        .await
        .unwrap_err();
    let read = service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: run.request.request_id.clone(),
            },
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(error, CloudStateError::InvalidEventPayload);
    assert_eq!(read.request.status, RequestStatus::Running);
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
async fn terminal_signal_rejects_invalid_json_payload_without_changing_request() {
    let mut service = CloudRequestService::default();
    let run = service
        .run(
            "caller-1",
            RequestRunParams {
                thread_id: Some("thread-1".to_string()),
                create_thread: None,
                input: "hello".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .unwrap();

    let error = service
        .terminal_signal(
            "caller-1",
            &run.request.request_id,
            TerminalSignal::Failed,
            Some("not-json".to_string()),
        )
        .await
        .unwrap_err();
    let read = service
        .read(
            "caller-1",
            RequestReadParams {
                request_id: run.request.request_id.clone(),
            },
        )
        .await
        .unwrap();
    let events = service
        .events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run.request.request_id,
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

    assert_eq!(error, CloudStateError::InvalidEventPayload);
    assert_eq!(read.request.status, RequestStatus::Running);
    assert_eq!(
        events
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );
}
