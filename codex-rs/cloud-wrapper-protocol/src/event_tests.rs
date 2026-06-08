use pretty_assertions::assert_eq;
use serde_json::json;

use crate::RequestEvent;
use crate::RequestEventsListParams;
use crate::RequestEventsListResponse;

#[test]
fn request_events_list_contract_serializes_with_camel_case_fields() {
    let params = RequestEventsListParams {
        request_id: "request-1".to_string(),
        cursor: Some(2),
        limit: Some(50),
    };
    let response = RequestEventsListResponse {
        data: vec![RequestEvent {
            request_id: "request-1".to_string(),
            sequence: 3,
            event_type: "turn/completed".to_string(),
            payload_inline: "{}".to_string(),
        }],
        next_cursor: None,
    };

    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({
            "requestId": "request-1",
            "cursor": 2,
            "limit": 50,
        })
    );
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "data": [
                {
                    "requestId": "request-1",
                    "sequence": 3,
                    "eventType": "turn/completed",
                    "payloadInline": "{}",
                }
            ],
            "nextCursor": null,
        })
    );
}
