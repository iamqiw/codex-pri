use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestEventsListParams {
    pub request_id: String,
    #[ts(optional = nullable)]
    pub cursor: Option<u64>,
    #[ts(optional = nullable)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestEvent {
    pub request_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub payload_inline: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestEventsListResponse {
    pub data: Vec<RequestEvent>,
    #[ts(optional = nullable)]
    pub next_cursor: Option<u64>,
}
