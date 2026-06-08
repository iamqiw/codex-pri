use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AppendThreadItemsWithLeaseParams {
    pub thread_id: String,
    pub request_id: String,
    pub lease_id: String,
    pub writer_owner_token: String,
    pub fencing_token: u64,
    pub append_idempotency_key: String,
    #[ts(optional = nullable)]
    pub expected_thread_version: Option<u64>,
    pub items: Vec<AppendThreadItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AppendThreadItem {
    pub payload_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AppendThreadItemsWithLeaseResponse {
    pub thread_version: u64,
    pub first_append_sequence: u64,
    pub last_append_sequence: u64,
    pub deduplicated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadItemsListParams {
    pub thread_id: String,
    #[ts(optional = nullable)]
    pub cursor: Option<u64>,
    #[ts(optional = nullable)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadItemRecord {
    pub thread_id: String,
    pub sequence: u64,
    pub payload_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadItemsListResponse {
    pub data: Vec<ThreadItemRecord>,
    pub next_cursor: Option<u64>,
}
