use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "v2/")]
pub enum RequestStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    OwnerTimedOut,
    LeaseLost,
    TurnTimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestRecord {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub status: RequestStatus,
    pub latest_event_cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestRunParams {
    #[ts(optional = nullable)]
    pub thread_id: Option<String>,
    #[ts(optional = nullable)]
    pub create_thread: Option<bool>,
    pub input: String,
    pub idempotency_key: String,
    #[ts(optional = nullable)]
    pub cwd: Option<String>,
    #[ts(optional = nullable)]
    pub client_info: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestRunResponse {
    pub request: RequestRecord,
    pub thread_id: String,
    pub event_cursor: u64,
    pub writer_lease: Option<WriterLeaseRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct WriterLeaseRecord {
    pub lease_id: String,
    pub writer_owner_token: String,
    pub fencing_token: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestReadParams {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestReadResponse {
    pub request: RequestRecord,
    pub latest_event_cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestCancelParams {
    pub request_id: String,
    #[ts(optional = nullable)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestCancelResponse {
    pub request: RequestRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestTerminalParams {
    pub request_id: String,
    pub signal: TerminalSignal,
    #[ts(optional = nullable)]
    pub payload_inline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RequestTerminalResponse {
    pub request: RequestRecord,
    pub latest_event_cursor: u64,
}

impl RequestStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Failed
                | Self::Cancelled
                | Self::Interrupted
                | Self::OwnerTimedOut
                | Self::LeaseLost
                | Self::TurnTimedOut
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStreamEvent {
    SseDisconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "v2/")]
pub enum TerminalSignal {
    CancelConfirmed,
    RuntimeWriteDenied,
    WriterLeaseLost,
    OwnerLeaseTimedOut,
    TurnTimedOut,
    Interrupted,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestStatusTransition {
    pub status: RequestStatus,
    pub changed: bool,
}

impl RequestStatusTransition {
    pub fn from_client_stream_event(
        current_status: RequestStatus,
        event: ClientStreamEvent,
    ) -> Self {
        match event {
            ClientStreamEvent::SseDisconnected => Self {
                status: current_status,
                changed: false,
            },
        }
    }

    pub fn from_terminal_signal(current_status: RequestStatus, signal: TerminalSignal) -> Self {
        if current_status.is_terminal() {
            return Self {
                status: current_status,
                changed: false,
            };
        }

        let status = match signal {
            TerminalSignal::Completed => RequestStatus::Completed,
            TerminalSignal::RuntimeWriteDenied | TerminalSignal::WriterLeaseLost => {
                RequestStatus::LeaseLost
            }
            TerminalSignal::OwnerLeaseTimedOut => RequestStatus::OwnerTimedOut,
            TerminalSignal::TurnTimedOut => RequestStatus::TurnTimedOut,
            TerminalSignal::CancelConfirmed => RequestStatus::Cancelled,
            TerminalSignal::Interrupted => RequestStatus::Interrupted,
            TerminalSignal::Failed => RequestStatus::Failed,
        };

        Self {
            status,
            changed: status != current_status,
        }
    }
}
