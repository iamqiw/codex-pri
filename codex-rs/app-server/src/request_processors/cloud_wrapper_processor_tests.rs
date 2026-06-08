use super::*;
use codex_cloud_wrapper_protocol::AppendThreadItem;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseParams;
use codex_cloud_wrapper_protocol::RequestCancelParams;
use codex_cloud_wrapper_protocol::RequestEventsListParams;
use codex_cloud_wrapper_protocol::RequestReadParams;
use codex_cloud_wrapper_protocol::RequestRunParams;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::RequestTerminalParams;
use codex_cloud_wrapper_protocol::TerminalSignal;
use codex_cloud_wrapper_protocol::ThreadItemRecord;
use codex_cloud_wrapper_protocol::ThreadItemsListParams;
use pretty_assertions::assert_eq;

use crate::INVALID_PARAMS_ERROR_CODE;

#[tokio::test]
async fn run_read_and_events_list_share_request_state() {
    let processor = CloudWrapperRequestProcessor::default();

    let run_response = processor
        .request_run(
            "caller-1",
            RequestRunParams {
                thread_id: None,
                create_thread: Some(true),
                input: "sha256:input-1".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request/run should succeed");

    let read_response = processor
        .request_read(
            "caller-1",
            RequestReadParams {
                request_id: run_response.request.request_id.clone(),
            },
        )
        .await
        .expect("request/read should succeed");
    let events_response = processor
        .request_events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run_response.request.request_id.clone(),
                cursor: None,
                limit: None,
            },
        )
        .await
        .expect("request/events/list should succeed");

    assert_eq!(read_response.request, run_response.request);
    assert_eq!(read_response.request.status, RequestStatus::Running);
    assert_eq!(
        events_response
            .data
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec!["request/queued".to_string(), "request/running".to_string()]
    );
}

#[tokio::test]
async fn request_operations_reject_a_different_caller_as_not_found() {
    let processor = CloudWrapperRequestProcessor::default();
    let run_response = processor
        .request_run(
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
        .expect("request/run should succeed");
    let writer_lease = run_response
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    let read_error = processor
        .request_read(
            "caller-2",
            RequestReadParams {
                request_id: run_response.request.request_id.clone(),
            },
        )
        .await
        .expect_err("different caller should not read request");
    let events_error = processor
        .request_events_list(
            "caller-2",
            RequestEventsListParams {
                request_id: run_response.request.request_id.clone(),
                cursor: None,
                limit: None,
            },
        )
        .await
        .expect_err("different caller should not read events");
    let append_error = processor
        .thread_append_with_lease(
            "caller-2",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id.clone(),
                request_id: run_response.request.request_id.clone(),
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
        .expect_err("different caller should not append thread items");
    let thread_items_error = processor
        .thread_items_list(
            "caller-2",
            ThreadItemsListParams {
                thread_id: run_response.thread_id,
                cursor: None,
                limit: None,
            },
        )
        .await
        .expect_err("different caller should not list thread items");
    let cancel_error = processor
        .request_cancel(
            "caller-2",
            RequestCancelParams {
                request_id: run_response.request.request_id.clone(),
                reason: Some("wrong caller".to_string()),
            },
        )
        .await
        .expect_err("different caller should not cancel request");
    let terminal_error = processor
        .request_terminal(
            "caller-2",
            RequestTerminalParams {
                request_id: run_response.request.request_id,
                signal: TerminalSignal::Completed,
                payload_inline: None,
            },
        )
        .await
        .expect_err("different caller should not mark terminal request");

    for error in [
        read_error,
        events_error,
        append_error,
        thread_items_error,
        cancel_error,
        terminal_error,
    ] {
        assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
        assert_eq!(error.message, "request was not found");
    }
}

#[tokio::test]
async fn append_with_lease_uses_writer_lease_from_request_run() {
    let processor = CloudWrapperRequestProcessor::default();

    let run_response = processor
        .request_run(
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
        .expect("request/run should succeed");
    let writer_lease = run_response
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    let first = processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id.clone(),
                request_id: run_response.request.request_id.clone(),
                lease_id: writer_lease.lease_id.clone(),
                writer_owner_token: writer_lease.writer_owner_token.clone(),
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
        .expect("thread/appendWithLease should succeed");
    let replay = processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id.clone(),
                request_id: run_response.request.request_id.clone(),
                lease_id: writer_lease.lease_id.clone(),
                writer_owner_token: writer_lease.writer_owner_token.clone(),
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
        .expect("thread/appendWithLease replay should deduplicate");
    let conflicting_replay_error = processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id.clone(),
                request_id: run_response.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: "object://items/different".to_string(),
                }],
            },
        )
        .await
        .expect_err("thread/appendWithLease conflicting replay should fail");
    let items = processor
        .thread_items_list(
            "caller-1",
            ThreadItemsListParams {
                thread_id: run_response.thread_id.clone(),
                cursor: None,
                limit: Some(1),
            },
        )
        .await
        .expect("thread/items/list should succeed");
    let second_page = processor
        .thread_items_list(
            "caller-1",
            ThreadItemsListParams {
                thread_id: run_response.thread_id,
                cursor: items.next_cursor,
                limit: Some(1),
            },
        )
        .await
        .expect("thread/items/list second page should succeed");

    assert_eq!(first.thread_version, 2);
    assert!(!first.deduplicated);
    assert_eq!(replay.thread_version, 2);
    assert!(replay.deduplicated);
    assert_eq!(conflicting_replay_error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        conflicting_replay_error.message,
        "idempotency key was reused with different input"
    );
    assert_eq!(
        items.data,
        vec![ThreadItemRecord {
            thread_id: "thread-1".to_string(),
            sequence: 1,
            payload_ref: "object://items/1".to_string(),
        }]
    );
    assert_eq!(items.next_cursor, Some(1));
    assert_eq!(
        second_page.data,
        vec![ThreadItemRecord {
            thread_id: "thread-1".to_string(),
            sequence: 2,
            payload_ref: "object://items/2".to_string(),
        }]
    );
    assert_eq!(second_page.next_cursor, None);
}

#[tokio::test]
async fn append_with_stale_expected_thread_version_maps_to_invalid_params() {
    let processor = CloudWrapperRequestProcessor::default();

    let run_response = processor
        .request_run(
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
        .expect("request/run should succeed");
    let writer_lease = run_response
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id.clone(),
                request_id: run_response.request.request_id.clone(),
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
        .expect("thread/appendWithLease should succeed");
    let error = processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id,
                request_id: run_response.request.request_id,
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
        .expect_err("stale expected thread version should be rejected");

    assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        error.message,
        "expected thread version does not match current thread version"
    );
}

#[tokio::test]
async fn append_with_stale_fencing_token_maps_to_invalid_params() {
    let processor = CloudWrapperRequestProcessor::default();

    let run_response = processor
        .request_run(
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
        .expect("request/run should succeed");
    let writer_lease = run_response
        .writer_lease
        .expect("running request should return writer lease");

    let error = processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id,
                request_id: run_response.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token + 1,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: vec![AppendThreadItem {
                    payload_ref: "object://items/1".to_string(),
                }],
            },
        )
        .await
        .expect_err("stale fencing token should be rejected");

    assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(error.message, "request lost its writer lease");
}

#[tokio::test]
async fn append_with_empty_items_maps_to_invalid_params() {
    let processor = CloudWrapperRequestProcessor::default();

    let run_response = processor
        .request_run(
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
        .expect("request/run should succeed");
    let writer_lease = run_response
        .writer_lease
        .expect("running request should return writer lease");

    let error = processor
        .thread_append_with_lease(
            "caller-1",
            AppendThreadItemsWithLeaseParams {
                thread_id: run_response.thread_id,
                request_id: run_response.request.request_id,
                lease_id: writer_lease.lease_id,
                writer_owner_token: writer_lease.writer_owner_token,
                fencing_token: writer_lease.fencing_token,
                append_idempotency_key: "append-1".to_string(),
                expected_thread_version: Some(0),
                items: Vec::new(),
            },
        )
        .await
        .expect_err("empty append should be rejected");

    assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        error.message,
        "append must include at least one thread item"
    );
}

#[tokio::test]
async fn cancel_marks_request_cancelled_and_appends_event() {
    let processor = CloudWrapperRequestProcessor::default();
    let run_response = processor
        .request_run(
            "caller-1",
            RequestRunParams {
                thread_id: None,
                create_thread: Some(true),
                input: "sha256:input-1".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request/run should succeed");

    let cancel_response = processor
        .request_cancel(
            "caller-1",
            RequestCancelParams {
                request_id: run_response.request.request_id.clone(),
                reason: Some("client cancelled".to_string()),
            },
        )
        .await
        .expect("request/cancel should succeed");
    let events_response = processor
        .request_events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run_response.request.request_id,
                cursor: Some(run_response.event_cursor),
                limit: None,
            },
        )
        .await
        .expect("request/events/list should succeed");

    assert_eq!(cancel_response.request.status, RequestStatus::Cancelled);
    assert_eq!(
        events_response
            .data
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec!["request/cancelled".to_string()]
    );
}

#[tokio::test]
async fn terminal_marks_request_completed_and_appends_event() {
    let processor = CloudWrapperRequestProcessor::default();
    let run_response = processor
        .request_run(
            "caller-1",
            RequestRunParams {
                thread_id: None,
                create_thread: Some(true),
                input: "sha256:input-1".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request/run should succeed");

    let terminal_response = processor
        .request_terminal(
            "caller-1",
            RequestTerminalParams {
                request_id: run_response.request.request_id.clone(),
                signal: TerminalSignal::Completed,
                payload_inline: None,
            },
        )
        .await
        .expect("request/terminal should succeed");
    let events_response = processor
        .request_events_list(
            "caller-1",
            RequestEventsListParams {
                request_id: run_response.request.request_id,
                cursor: Some(run_response.event_cursor),
                limit: None,
            },
        )
        .await
        .expect("request/events/list should succeed");

    assert_eq!(terminal_response.request.status, RequestStatus::Completed);
    assert_eq!(
        events_response
            .data
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec!["request/completed".to_string()]
    );
}

#[tokio::test]
async fn terminal_invalid_payload_maps_to_invalid_params() {
    let processor = CloudWrapperRequestProcessor::default();
    let run_response = processor
        .request_run(
            "caller-1",
            RequestRunParams {
                thread_id: None,
                create_thread: Some(true),
                input: "sha256:input-1".to_string(),
                idempotency_key: "idem-1".to_string(),
                cwd: None,
                client_info: None,
            },
        )
        .await
        .expect("request/run should succeed");

    let error = processor
        .request_terminal(
            "caller-1",
            RequestTerminalParams {
                request_id: run_response.request.request_id,
                signal: TerminalSignal::Failed,
                payload_inline: Some("not-json".to_string()),
            },
        )
        .await
        .expect_err("invalid payload should be rejected");

    assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(error.message, "request event payload must be valid JSON");
}

#[tokio::test]
async fn idempotency_conflict_maps_to_invalid_params() {
    let processor = CloudWrapperRequestProcessor::default();
    let params = RequestRunParams {
        thread_id: Some("thread-1".to_string()),
        create_thread: None,
        input: "sha256:input-1".to_string(),
        idempotency_key: "idem-1".to_string(),
        cwd: None,
        client_info: None,
    };

    processor
        .request_run("caller-1", params.clone())
        .await
        .expect("first request/run should succeed");
    let error = processor
        .request_run(
            "caller-1",
            RequestRunParams {
                input: "sha256:input-2".to_string(),
                ..params
            },
        )
        .await
        .expect_err("conflicting idempotency key should be rejected");

    assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        error.message,
        "idempotency key was reused with different input"
    );
}

#[test]
fn request_not_queued_maps_to_invalid_params() {
    let error = cloud_state_error(CloudStateError::RequestNotQueued);

    assert_eq!(error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        error.message,
        "request is not queued and cannot acquire a writer lease"
    );
}
