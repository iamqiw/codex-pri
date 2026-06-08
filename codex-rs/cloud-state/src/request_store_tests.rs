use pretty_assertions::assert_eq;

use crate::CloudStateError;
use crate::CreateRequestParams;
use crate::EventAppendParams;
use crate::EventRecord;
use crate::InMemoryCloudStateStore;
use crate::LeaseAcquireOutcome;
use crate::LeaseAppendParams;
use crate::TerminalStatusUpdate;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::ThreadItemRecord;

#[test]
fn create_request_is_idempotent_for_same_caller_thread_key_and_input_hash() {
    let mut store = InMemoryCloudStateStore::default();
    let params = CreateRequestParams {
        caller_id: "caller-1".to_string(),
        thread_id: "thread-1".to_string(),
        idempotency_key: "idem-1".to_string(),
        input_hash: "hash-a".to_string(),
    };

    let first = store.create_request(params.clone()).unwrap();
    let second = store.create_request(params).unwrap();

    assert_eq!(second, first);
    assert_eq!(first.status, RequestStatus::Queued);
}

#[test]
fn create_request_rejects_same_idempotency_key_with_different_input_hash() {
    let mut store = InMemoryCloudStateStore::default();
    let first = CreateRequestParams {
        caller_id: "caller-1".to_string(),
        thread_id: "thread-1".to_string(),
        idempotency_key: "idem-1".to_string(),
        input_hash: "hash-a".to_string(),
    };
    let conflicting = CreateRequestParams {
        input_hash: "hash-b".to_string(),
        ..first.clone()
    };

    store.create_request(first).unwrap();
    let error = store.create_request(conflicting).unwrap_err();

    assert_eq!(error, CloudStateError::IdempotencyConflict);
}

#[test]
fn acquire_thread_writer_lease_serializes_running_requests_by_thread() {
    let mut store = InMemoryCloudStateStore::default();
    let first = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let second = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();

    let first_outcome = store
        .acquire_thread_writer_lease(&first.request_id, "owner-1")
        .unwrap();
    let second_outcome = store
        .acquire_thread_writer_lease(&second.request_id, "owner-2")
        .unwrap();

    assert_eq!(
        first_outcome,
        LeaseAcquireOutcome::Acquired {
            lease_id: "lease-1".to_string(),
            fencing_token: 1,
        }
    );
    assert_eq!(second_outcome, LeaseAcquireOutcome::Queued);
    assert_eq!(
        store.read_request(&first.request_id).unwrap().status,
        RequestStatus::Running
    );
    assert_eq!(
        store.read_request(&second.request_id).unwrap().status,
        RequestStatus::Queued
    );
}

#[test]
fn terminal_queued_request_does_not_release_running_request_lease() {
    let mut store = InMemoryCloudStateStore::default();
    let running = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let queued = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = store
        .acquire_thread_writer_lease(&running.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected first request to acquire lease");
    };

    assert_eq!(
        store
            .mark_request_terminal(&queued.request_id, RequestStatus::Cancelled)
            .unwrap(),
        TerminalStatusUpdate::Changed
    );
    let append = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: running.request_id.clone(),
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .unwrap();

    assert_eq!(append.first_append_sequence, 1);
    assert_eq!(
        store.read_request(&running.request_id).unwrap().status,
        RequestStatus::Running
    );
    assert_eq!(
        store.read_request(&queued.request_id).unwrap().status,
        RequestStatus::Cancelled
    );
}

#[test]
fn append_with_wrong_fencing_token_is_rejected_and_marks_request_lease_lost() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let queued = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired { lease_id, .. } = store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected first request to acquire lease");
    };

    let error = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id.clone(),
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token: 99,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .unwrap_err();

    assert_eq!(error, CloudStateError::LeaseLost);
    assert_eq!(
        store.read_request(&request.request_id).unwrap().status,
        RequestStatus::LeaseLost
    );
    assert_eq!(
        store
            .acquire_thread_writer_lease(&queued.request_id, "owner-2")
            .unwrap(),
        LeaseAcquireOutcome::Acquired {
            lease_id: "lease-2".to_string(),
            fencing_token: 2,
        }
    );
}

#[test]
fn append_from_queued_request_without_held_lease_does_not_mark_it_lease_lost() {
    let mut store = InMemoryCloudStateStore::default();
    let running = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let queued = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = store
        .acquire_thread_writer_lease(&running.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected first request to acquire lease");
    };

    let error = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: queued.request_id.clone(),
            lease_id: "lease-missing".to_string(),
            writer_owner_token: "owner-2".to_string(),
            fencing_token: 99,
            append_idempotency_key: "append-queued".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/queued".to_string()],
        })
        .unwrap_err();
    let append = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: running.request_id.clone(),
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-running".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/running".to_string()],
        })
        .unwrap();

    assert_eq!(error, CloudStateError::LeaseLost);
    assert_eq!(
        store.read_request(&queued.request_id).unwrap().status,
        RequestStatus::Queued
    );
    assert_eq!(
        store.read_request(&running.request_id).unwrap().status,
        RequestStatus::Running
    );
    assert_eq!(append.first_append_sequence, 1);
}

#[test]
fn append_with_current_lease_allocates_sequences_and_deduplicates_retries() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected first request to acquire lease");
    };
    let params = LeaseAppendParams {
        request_id: request.request_id,
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

    let first = store
        .append_thread_items_with_lease(params.clone())
        .unwrap();
    let second = store
        .append_thread_items_with_lease(params.clone())
        .unwrap();
    let mut conflicting_replay = params;
    conflicting_replay.payload_refs = vec!["object://items/different".to_string()];
    let error = store
        .append_thread_items_with_lease(conflicting_replay)
        .unwrap_err();

    assert_eq!(first.thread_version, 2);
    assert_eq!(first.first_append_sequence, 1);
    assert_eq!(first.last_append_sequence, 2);
    assert_eq!(first.deduplicated, false);
    assert_eq!(second.thread_version, 2);
    assert_eq!(second.first_append_sequence, 1);
    assert_eq!(second.last_append_sequence, 2);
    assert_eq!(second.deduplicated, true);
    assert_eq!(error, CloudStateError::IdempotencyConflict);
    let (items, next_cursor) = store
        .list_thread_items("thread-1", /*cursor*/ None, /*limit*/ 10)
        .unwrap();
    assert_eq!(
        items,
        vec![
            ThreadItemRecord {
                thread_id: "thread-1".to_string(),
                sequence: 1,
                payload_ref: "object://items/1".to_string(),
            },
            ThreadItemRecord {
                thread_id: "thread-1".to_string(),
                sequence: 2,
                payload_ref: "object://items/2".to_string(),
            },
        ]
    );
    assert_eq!(next_cursor, None);
}

#[test]
fn append_replay_with_wrong_fencing_token_is_rejected_before_dedupe() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected first request to acquire lease");
    };
    let params = LeaseAppendParams {
        request_id: request.request_id.clone(),
        lease_id,
        writer_owner_token: "owner-1".to_string(),
        fencing_token,
        append_idempotency_key: "append-1".to_string(),
        expected_thread_version: Some(0),
        payload_refs: vec!["object://items/1".to_string()],
    };

    store
        .append_thread_items_with_lease(params.clone())
        .unwrap();
    let mut replay_with_wrong_fencing_token = params;
    replay_with_wrong_fencing_token.fencing_token = 99;
    let error = store
        .append_thread_items_with_lease(replay_with_wrong_fencing_token)
        .unwrap_err();

    assert_eq!(error, CloudStateError::LeaseLost);
}

#[test]
fn append_idempotency_key_replay_is_scoped_to_original_request() {
    let mut store = InMemoryCloudStateStore::default();
    let first_request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let second_request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id: first_lease_id,
        fencing_token: first_fencing_token,
    } = store
        .acquire_thread_writer_lease(&first_request.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected first request to acquire lease");
    };
    store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: first_request.request_id.clone(),
            lease_id: first_lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token: first_fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .unwrap();
    store
        .mark_request_terminal(&first_request.request_id, RequestStatus::Completed)
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id: second_lease_id,
        fencing_token: second_fencing_token,
    } = store
        .acquire_thread_writer_lease(&second_request.request_id, "owner-2")
        .unwrap()
    else {
        panic!("expected second request to acquire lease");
    };

    let error = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: second_request.request_id,
            lease_id: second_lease_id,
            writer_owner_token: "owner-2".to_string(),
            fencing_token: second_fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(1),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .unwrap_err();

    assert_eq!(error, CloudStateError::IdempotencyConflict);
}

#[test]
fn append_rejects_stale_expected_thread_version() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected request to acquire lease");
    };

    store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id.clone(),
            lease_id: lease_id.clone(),
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/1".to_string()],
        })
        .unwrap();
    let error = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id,
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-2".to_string(),
            expected_thread_version: Some(0),
            payload_refs: vec!["object://items/2".to_string()],
        })
        .unwrap_err();

    assert_eq!(error, CloudStateError::ThreadVersionConflict);
}

#[test]
fn append_rejects_empty_payload_refs() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let LeaseAcquireOutcome::Acquired {
        lease_id,
        fencing_token,
    } = store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap()
    else {
        panic!("expected request to acquire lease");
    };

    let error = store
        .append_thread_items_with_lease(LeaseAppendParams {
            request_id: request.request_id,
            lease_id,
            writer_owner_token: "owner-1".to_string(),
            fencing_token,
            append_idempotency_key: "append-1".to_string(),
            expected_thread_version: Some(0),
            payload_refs: Vec::new(),
        })
        .unwrap_err();

    assert_eq!(error, CloudStateError::EmptyAppend);
}

#[test]
fn terminal_request_releases_thread_writer_lease_for_next_queued_request() {
    let mut store = InMemoryCloudStateStore::default();
    let first = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let second = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();

    assert_eq!(
        store
            .acquire_thread_writer_lease(&first.request_id, "owner-1")
            .unwrap(),
        LeaseAcquireOutcome::Acquired {
            lease_id: "lease-1".to_string(),
            fencing_token: 1,
        }
    );
    assert_eq!(
        store
            .acquire_thread_writer_lease(&second.request_id, "owner-2")
            .unwrap(),
        LeaseAcquireOutcome::Queued
    );

    store
        .mark_request_terminal(&first.request_id, RequestStatus::Completed)
        .unwrap();

    assert_eq!(
        store
            .acquire_thread_writer_lease(&second.request_id, "owner-2")
            .unwrap(),
        LeaseAcquireOutcome::Acquired {
            lease_id: "lease-2".to_string(),
            fencing_token: 2,
        }
    );
    assert_eq!(
        store.read_request(&first.request_id).unwrap().status,
        RequestStatus::Completed
    );
    assert_eq!(
        store.read_request(&second.request_id).unwrap().status,
        RequestStatus::Running
    );
}

#[test]
fn owner_timed_out_request_releases_lease_for_next_queued_request() {
    let mut store = InMemoryCloudStateStore::default();
    let first = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    let second = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-2".to_string(),
            input_hash: "hash-b".to_string(),
        })
        .unwrap();

    assert_eq!(
        store
            .acquire_thread_writer_lease(&first.request_id, "owner-1")
            .unwrap(),
        LeaseAcquireOutcome::Acquired {
            lease_id: "lease-1".to_string(),
            fencing_token: 1,
        }
    );
    assert_eq!(
        store
            .acquire_thread_writer_lease(&second.request_id, "owner-2")
            .unwrap(),
        LeaseAcquireOutcome::Queued
    );

    store
        .mark_request_terminal(&first.request_id, RequestStatus::OwnerTimedOut)
        .unwrap();

    assert_eq!(
        store
            .acquire_thread_writer_lease(&second.request_id, "owner-2")
            .unwrap(),
        LeaseAcquireOutcome::Acquired {
            lease_id: "lease-2".to_string(),
            fencing_token: 2,
        }
    );
    assert_eq!(
        store.read_request(&first.request_id).unwrap().status,
        RequestStatus::OwnerTimedOut
    );
    assert_eq!(
        store.read_request(&second.request_id).unwrap().status,
        RequestStatus::Running
    );
}

#[test]
fn terminal_request_cannot_reacquire_writer_lease() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();

    store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap();
    store
        .mark_request_terminal(&request.request_id, RequestStatus::OwnerTimedOut)
        .unwrap();

    assert_eq!(
        store
            .acquire_thread_writer_lease(&request.request_id, "owner-1")
            .unwrap_err(),
        CloudStateError::RequestNotQueued
    );
    assert_eq!(
        store.read_request(&request.request_id).unwrap().status,
        RequestStatus::OwnerTimedOut
    );
}

#[test]
fn terminal_request_is_not_overwritten_by_later_terminal_status() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();

    store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap();
    let first_update = store
        .mark_request_terminal(&request.request_id, RequestStatus::OwnerTimedOut)
        .unwrap();
    let second_update = store
        .mark_request_terminal(&request.request_id, RequestStatus::Failed)
        .unwrap();

    assert_eq!(first_update, TerminalStatusUpdate::Changed);
    assert_eq!(second_update, TerminalStatusUpdate::Unchanged);
    assert_eq!(
        store.read_request(&request.request_id).unwrap().status,
        RequestStatus::OwnerTimedOut
    );
}

#[test]
fn terminal_request_with_event_updates_status_cursor_and_event_together() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();

    store
        .acquire_thread_writer_lease(&request.request_id, "owner-1")
        .unwrap();
    let update = store
        .mark_request_terminal_with_event(
            EventAppendParams {
                request_id: request.request_id.clone(),
                event_type: "request/completed".to_string(),
                payload_inline: "{\"ok\":true}".to_string(),
            },
            RequestStatus::Completed,
        )
        .unwrap();
    let persisted = store.read_request(&request.request_id).unwrap();
    let events = store
        .list_request_events(&request.request_id, /*cursor*/ None, /*limit*/ 10)
        .unwrap();

    assert_eq!(update, TerminalStatusUpdate::Changed);
    assert_eq!(persisted.status, RequestStatus::Completed);
    assert_eq!(persisted.latest_event_cursor, 1);
    assert_eq!(
        store.read_thread_writer_lease(&request.request_id).unwrap(),
        None
    );
    assert_eq!(
        events.data,
        vec![EventRecord {
            request_id: request.request_id,
            sequence: 1,
            event_type: "request/completed".to_string(),
            payload_inline: "{\"ok\":true}".to_string(),
        }]
    );
}

#[test]
fn request_events_are_persisted_and_read_by_cursor_in_sequence_order() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();

    store
        .append_request_event(EventAppendParams {
            request_id: request.request_id.clone(),
            event_type: "request/queued".to_string(),
            payload_inline: "{}".to_string(),
        })
        .unwrap();
    store
        .append_request_event(EventAppendParams {
            request_id: request.request_id.clone(),
            event_type: "turn/started".to_string(),
            payload_inline: "{\"turnId\":\"turn-1\"}".to_string(),
        })
        .unwrap();
    store
        .append_request_event(EventAppendParams {
            request_id: request.request_id.clone(),
            event_type: "turn/completed".to_string(),
            payload_inline: "{}".to_string(),
        })
        .unwrap();

    let first_page = store
        .list_request_events(&request.request_id, /*cursor*/ None, /*limit*/ 2)
        .unwrap();
    let second_page = store
        .list_request_events(
            &request.request_id,
            first_page.next_cursor,
            /*limit*/ 2,
        )
        .unwrap();

    assert_eq!(
        first_page
            .data
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        first_page
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "turn/started"]
    );
    assert_eq!(first_page.next_cursor, Some(2));
    assert_eq!(
        second_page
            .data
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![3]
    );
    assert_eq!(second_page.next_cursor, None);
    assert_eq!(
        store
            .read_request(&request.request_id)
            .unwrap()
            .latest_event_cursor,
        3
    );
}

#[test]
fn request_event_append_rejects_invalid_json_payload_without_advancing_cursor() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();

    let error = store
        .append_request_event(EventAppendParams {
            request_id: request.request_id.clone(),
            event_type: "request/invalid".to_string(),
            payload_inline: "not-json".to_string(),
        })
        .unwrap_err();

    assert_eq!(error, CloudStateError::InvalidEventPayload);
    assert_eq!(
        store
            .read_request(&request.request_id)
            .unwrap()
            .latest_event_cursor,
        0
    );
}

#[test]
fn request_record_projects_to_wrapper_protocol_record() {
    let mut store = InMemoryCloudStateStore::default();
    let request = store
        .create_request(CreateRequestParams {
            caller_id: "caller-1".to_string(),
            thread_id: "thread-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            input_hash: "hash-a".to_string(),
        })
        .unwrap();
    store
        .append_request_event(EventAppendParams {
            request_id: request.request_id.clone(),
            event_type: "request/queued".to_string(),
            payload_inline: "{}".to_string(),
        })
        .unwrap();

    let projected = store
        .read_request(&request.request_id)
        .unwrap()
        .to_protocol();

    assert_eq!(
        projected,
        codex_cloud_wrapper_protocol::RequestRecord {
            request_id: "request-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: None,
            status: RequestStatus::Queued,
            latest_event_cursor: 1,
        }
    );
}
