use pretty_assertions::assert_eq;
use serde_json::json;

use crate::AppendThreadItem;
use crate::AppendThreadItemsWithLeaseParams;
use crate::AppendThreadItemsWithLeaseResponse;
use crate::CloudRuntimeDefaults;
use crate::ThreadItemRecord;
use crate::ThreadItemsListParams;
use crate::ThreadItemsListResponse;

#[test]
fn cloud_runtime_defaults_match_stateless_design_decisions() {
    assert_eq!(
        CloudRuntimeDefaults::default(),
        CloudRuntimeDefaults {
            mysql_max_connections: 20,
            thread_writer_lease_ttl_ms: 30_000,
            thread_writer_lease_renew_interval_ms: 2_000,
            owner_lease_ttl_ms: 180_000,
            owner_lease_heartbeat_ms: 2_000,
            turn_timeout_ms: 1_800_000,
            request_timeout_ms: 60_000,
        }
    );
}

#[test]
fn append_with_lease_response_exposes_thread_version_sequence_range_and_deduplication() {
    let response = AppendThreadItemsWithLeaseResponse {
        thread_version: 13,
        first_append_sequence: 8,
        last_append_sequence: 9,
        deduplicated: false,
    };

    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "threadVersion": 13,
            "firstAppendSequence": 8,
            "lastAppendSequence": 9,
            "deduplicated": false,
        })
    );
}

#[test]
fn append_with_lease_params_serialize_as_camel_case_contract() {
    let params = AppendThreadItemsWithLeaseParams {
        thread_id: "thread-1".to_string(),
        request_id: "request-1".to_string(),
        lease_id: "lease-1".to_string(),
        writer_owner_token: "owner-token".to_string(),
        fencing_token: 7,
        append_idempotency_key: "append-1".to_string(),
        expected_thread_version: Some(12),
        items: vec![AppendThreadItem {
            payload_ref: "object://items/1".to_string(),
        }],
    };

    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({
            "threadId": "thread-1",
            "requestId": "request-1",
            "leaseId": "lease-1",
            "writerOwnerToken": "owner-token",
            "fencingToken": 7,
            "appendIdempotencyKey": "append-1",
            "expectedThreadVersion": 12,
            "items": [
                {
                    "payloadRef": "object://items/1",
                }
            ],
        })
    );
}

#[test]
fn thread_items_list_contract_serializes_as_camel_case() {
    let params = ThreadItemsListParams {
        thread_id: "thread-1".to_string(),
        cursor: Some(6),
        limit: Some(50),
    };
    let response = ThreadItemsListResponse {
        data: vec![ThreadItemRecord {
            thread_id: "thread-1".to_string(),
            sequence: 7,
            payload_ref: "object://items/7".to_string(),
        }],
        next_cursor: Some(7),
    };

    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({
            "threadId": "thread-1",
            "cursor": 6,
            "limit": 50,
        })
    );
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "data": [
                {
                    "threadId": "thread-1",
                    "sequence": 7,
                    "payloadRef": "object://items/7",
                }
            ],
            "nextCursor": 7,
        })
    );
}
