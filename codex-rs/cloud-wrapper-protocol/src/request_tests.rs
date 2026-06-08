use pretty_assertions::assert_eq;
use serde_json::json;

use crate::ClientStreamEvent;
use crate::RequestCancelParams;
use crate::RequestReadParams;
use crate::RequestReadResponse;
use crate::RequestRecord;
use crate::RequestRunParams;
use crate::RequestRunResponse;
use crate::RequestStatus;
use crate::RequestStatusTransition;
use crate::RequestTerminalParams;
use crate::RequestTerminalResponse;
use crate::TerminalSignal;

#[test]
fn sse_disconnect_does_not_change_running_request_status() {
    let transition = RequestStatusTransition::from_client_stream_event(
        RequestStatus::Running,
        ClientStreamEvent::SseDisconnected,
    );

    assert_eq!(
        transition,
        RequestStatusTransition {
            status: RequestStatus::Running,
            changed: false,
        }
    );
}

#[test]
fn persisted_completion_is_not_overwritten_by_later_terminal_signals() {
    let transition = RequestStatusTransition::from_terminal_signal(
        RequestStatus::Completed,
        TerminalSignal::OwnerLeaseTimedOut,
    );

    assert_eq!(
        transition,
        RequestStatusTransition {
            status: RequestStatus::Completed,
            changed: false,
        }
    );
}

#[test]
fn persisted_terminal_status_is_not_overwritten_by_later_terminal_signals() {
    let transition = RequestStatusTransition::from_terminal_signal(
        RequestStatus::Cancelled,
        TerminalSignal::Failed,
    );

    assert_eq!(
        transition,
        RequestStatusTransition {
            status: RequestStatus::Cancelled,
            changed: false,
        }
    );
}

#[test]
fn runtime_write_denied_terminates_as_lease_lost() {
    let transition = RequestStatusTransition::from_terminal_signal(
        RequestStatus::Running,
        TerminalSignal::RuntimeWriteDenied,
    );

    assert_eq!(
        transition,
        RequestStatusTransition {
            status: RequestStatus::LeaseLost,
            changed: true,
        }
    );
}

#[test]
fn request_status_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&RequestStatus::OwnerTimedOut).unwrap(),
        "\"owner_timed_out\""
    );
    assert_eq!(
        serde_json::to_string(&RequestStatus::LeaseLost).unwrap(),
        "\"lease_lost\""
    );
}

#[test]
fn request_run_read_and_cancel_contracts_serialize_with_camel_case_fields() {
    let request = RequestRecord {
        request_id: "request-1".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: Some("turn-1".to_string()),
        status: RequestStatus::Queued,
        latest_event_cursor: 0,
    };

    assert_eq!(
        serde_json::to_value(RequestRunParams {
            thread_id: Some("thread-1".to_string()),
            create_thread: None,
            input: "hello".to_string(),
            idempotency_key: "idem-1".to_string(),
            cwd: Some("/workspace".to_string()),
            client_info: Some("integration-test".to_string()),
        })
        .unwrap(),
        json!({
            "threadId": "thread-1",
            "createThread": null,
            "input": "hello",
            "idempotencyKey": "idem-1",
            "cwd": "/workspace",
            "clientInfo": "integration-test",
        })
    );
    assert_eq!(
        serde_json::to_value(RequestRunResponse {
            request: request.clone(),
            thread_id: "thread-1".to_string(),
            event_cursor: 0,
            writer_lease: Some(crate::WriterLeaseRecord {
                lease_id: "lease-1".to_string(),
                writer_owner_token: "owner-token".to_string(),
                fencing_token: 7,
            }),
        })
        .unwrap(),
        json!({
            "request": {
                "requestId": "request-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "status": "queued",
                "latestEventCursor": 0,
            },
            "threadId": "thread-1",
            "eventCursor": 0,
            "writerLease": {
                "leaseId": "lease-1",
                "writerOwnerToken": "owner-token",
                "fencingToken": 7,
            },
        })
    );
    assert_eq!(
        serde_json::to_value(RequestReadParams {
            request_id: "request-1".to_string(),
        })
        .unwrap(),
        json!({
            "requestId": "request-1",
        })
    );
    assert_eq!(
        serde_json::to_value(RequestReadResponse {
            request,
            latest_event_cursor: 0,
        })
        .unwrap(),
        json!({
            "request": {
                "requestId": "request-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "status": "queued",
                "latestEventCursor": 0,
            },
            "latestEventCursor": 0,
        })
    );
    assert_eq!(
        serde_json::to_value(RequestCancelParams {
            request_id: "request-1".to_string(),
            reason: Some("user_cancelled".to_string()),
        })
        .unwrap(),
        json!({
            "requestId": "request-1",
            "reason": "user_cancelled",
        })
    );
    assert_eq!(
        serde_json::to_value(RequestTerminalParams {
            request_id: "request-1".to_string(),
            signal: TerminalSignal::OwnerLeaseTimedOut,
            payload_inline: Some(r#"{"owner":"worker-1"}"#.to_string()),
        })
        .unwrap(),
        json!({
            "requestId": "request-1",
            "signal": "owner_lease_timed_out",
            "payloadInline": "{\"owner\":\"worker-1\"}",
        })
    );
    assert_eq!(
        serde_json::to_value(RequestTerminalResponse {
            request: RequestRecord {
                request_id: "request-1".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                status: RequestStatus::OwnerTimedOut,
                latest_event_cursor: 3,
            },
            latest_event_cursor: 3,
        })
        .unwrap(),
        json!({
            "request": {
                "requestId": "request-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "status": "owner_timed_out",
                "latestEventCursor": 3,
            },
            "latestEventCursor": 3,
        })
    );
}
