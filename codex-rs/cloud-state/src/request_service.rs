use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseParams;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseResponse;
use codex_cloud_wrapper_protocol::RequestCancelParams;
use codex_cloud_wrapper_protocol::RequestCancelResponse;
use codex_cloud_wrapper_protocol::RequestEvent;
use codex_cloud_wrapper_protocol::RequestEventsListParams;
use codex_cloud_wrapper_protocol::RequestEventsListResponse;
use codex_cloud_wrapper_protocol::RequestReadParams;
use codex_cloud_wrapper_protocol::RequestReadResponse;
use codex_cloud_wrapper_protocol::RequestRunParams;
use codex_cloud_wrapper_protocol::RequestRunResponse;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::RequestStatusTransition;
use codex_cloud_wrapper_protocol::RequestTerminalParams;
use codex_cloud_wrapper_protocol::RequestTerminalResponse;
use codex_cloud_wrapper_protocol::TerminalSignal;
use codex_cloud_wrapper_protocol::ThreadItemsListParams;
use codex_cloud_wrapper_protocol::ThreadItemsListResponse;

use crate::CloudStateError;
use crate::CloudStateStore;
use crate::ConfigSnapshotRecord;
use crate::CreateRequestParams;
use crate::EventAppendParams;
use crate::EventsPage;
use crate::InMemoryCloudStateStore;
use crate::LeaseAcquireOutcome;
use crate::MysqlCloudStateStore;
use crate::StateMetadataRecord;

const REQUEST_EVENTS_LIST_DEFAULT_LIMIT: usize = 100;
const REQUEST_EVENTS_LIST_MAX_LIMIT: usize = 100;
const THREAD_ITEMS_LIST_DEFAULT_LIMIT: usize = 100;
const THREAD_ITEMS_LIST_MAX_LIMIT: usize = 100;

#[derive(Debug)]
pub struct CloudRequestService<S = InMemoryCloudStateStore> {
    store: S,
    next_thread_sequence: u64,
}

impl Default for CloudRequestService<InMemoryCloudStateStore> {
    fn default() -> Self {
        Self::new(InMemoryCloudStateStore::default())
    }
}

impl<S> CloudRequestService<S>
where
    S: CloudStateStore,
{
    pub fn new(store: S) -> Self {
        Self {
            store,
            next_thread_sequence: 0,
        }
    }

    pub async fn run(
        &mut self,
        caller_id: &str,
        params: RequestRunParams,
    ) -> Result<RequestRunResponse, CloudStateError> {
        let thread_id = self.resolve_thread_id(params.thread_id, params.create_thread);
        let record = self
            .store
            .create_request(CreateRequestParams {
                caller_id: caller_id.to_string(),
                thread_id: thread_id.clone(),
                idempotency_key: params.idempotency_key,
                input_hash: params.input,
            })
            .await?;

        if record.latest_event_cursor == 0 {
            self.store
                .append_request_event_if_latest_cursor(
                    EventAppendParams {
                        request_id: record.request_id.clone(),
                        event_type: "request/queued".to_string(),
                        payload_inline: "{}".to_string(),
                    },
                    /*expected_latest_event_cursor*/ 0,
                )
                .await?;
        }

        let record = self.store.read_request(&record.request_id).await?;
        if record.status == RequestStatus::Queued
            && matches!(
                self.store
                    .acquire_thread_writer_lease(&record.request_id, caller_id)
                    .await?,
                LeaseAcquireOutcome::Acquired { .. }
            )
        {
            self.store
                .append_request_event(EventAppendParams {
                    request_id: record.request_id.clone(),
                    event_type: "request/running".to_string(),
                    payload_inline: "{}".to_string(),
                })
                .await?;
        }

        let request = self.store.read_request(&record.request_id).await?;
        let writer_lease = if request.status == RequestStatus::Running {
            self.store
                .read_thread_writer_lease(&request.request_id)
                .await?
        } else {
            None
        };
        Ok(RequestRunResponse {
            request: request.to_protocol(),
            thread_id,
            event_cursor: request.latest_event_cursor,
            writer_lease,
        })
    }

    pub async fn append_thread_items_with_lease(
        &mut self,
        caller_id: &str,
        params: AppendThreadItemsWithLeaseParams,
    ) -> Result<AppendThreadItemsWithLeaseResponse, CloudStateError> {
        let request = self
            .read_request_for_caller(caller_id, &params.request_id)
            .await?;
        if request.thread_id != params.thread_id {
            return Err(CloudStateError::ThreadMismatch);
        }
        self.store
            .append_thread_items_with_lease(crate::LeaseAppendParams {
                request_id: params.request_id,
                lease_id: params.lease_id,
                writer_owner_token: params.writer_owner_token,
                fencing_token: params.fencing_token,
                append_idempotency_key: params.append_idempotency_key,
                expected_thread_version: params.expected_thread_version,
                payload_refs: params
                    .items
                    .into_iter()
                    .map(|item| item.payload_ref)
                    .collect(),
            })
            .await
    }

    pub async fn thread_items_list(
        &self,
        caller_id: &str,
        params: ThreadItemsListParams,
    ) -> Result<ThreadItemsListResponse, CloudStateError> {
        self.ensure_caller_can_access_thread(caller_id, &params.thread_id)
            .await?;
        let (data, next_cursor) = self
            .store
            .list_thread_items(
                &params.thread_id,
                params.cursor,
                thread_items_list_limit(params.limit),
            )
            .await?;
        Ok(ThreadItemsListResponse { data, next_cursor })
    }

    pub async fn read(
        &self,
        caller_id: &str,
        params: RequestReadParams,
    ) -> Result<RequestReadResponse, CloudStateError> {
        let request = self
            .read_request_for_caller(caller_id, &params.request_id)
            .await?;
        Ok(RequestReadResponse {
            latest_event_cursor: request.latest_event_cursor,
            request: request.to_protocol(),
        })
    }

    pub async fn set_turn_id(
        &mut self,
        caller_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<crate::RequestRecord, CloudStateError> {
        self.read_request_for_caller(caller_id, request_id).await?;
        self.store.set_request_turn_id(request_id, turn_id).await
    }

    pub async fn renew_request_leases(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<bool, CloudStateError> {
        self.store
            .renew_request_leases(request_id, owner_instance_id)
            .await
    }

    pub async fn expire_owner_leases(&mut self) -> Result<usize, CloudStateError> {
        self.store.expire_owner_leases().await
    }

    pub async fn cancel(
        &mut self,
        caller_id: &str,
        params: RequestCancelParams,
    ) -> Result<RequestCancelResponse, CloudStateError> {
        let payload_inline = params.reason.map_or_else(
            || "{}".to_string(),
            |reason| serde_json::json!({ "reason": reason }).to_string(),
        );
        let response = self
            .terminal_signal(
                caller_id,
                &params.request_id,
                TerminalSignal::CancelConfirmed,
                Some(payload_inline),
            )
            .await?;
        Ok(RequestCancelResponse {
            request: response.request,
        })
    }

    pub async fn terminal_signal(
        &mut self,
        caller_id: &str,
        request_id: &str,
        signal: TerminalSignal,
        payload_inline: Option<String>,
    ) -> Result<RequestTerminalResponse, CloudStateError> {
        let current = self.read_request_for_caller(caller_id, request_id).await?;
        let transition = RequestStatusTransition::from_terminal_signal(current.status, signal);
        if transition.changed {
            if let Some(payload_inline) = payload_inline.as_deref() {
                serde_json::from_str::<serde_json::Value>(payload_inline)
                    .map_err(|_| CloudStateError::InvalidEventPayload)?;
            }
            let event_type = match transition.status {
                RequestStatus::Completed => "request/completed",
                RequestStatus::Failed => "request/failed",
                RequestStatus::Cancelled => "request/cancelled",
                RequestStatus::Interrupted => "request/interrupted",
                RequestStatus::OwnerTimedOut => "request/owner_timed_out",
                RequestStatus::LeaseLost => "request/lease_lost",
                RequestStatus::TurnTimedOut => "request/turn_timed_out",
                RequestStatus::Queued | RequestStatus::Running | RequestStatus::Cancelling => {
                    unreachable!("terminal signal must map to terminal status")
                }
            };
            let update = self
                .store
                .mark_request_terminal_with_event(
                    EventAppendParams {
                        request_id: request_id.to_string(),
                        event_type: event_type.to_string(),
                        payload_inline: payload_inline.unwrap_or_else(|| "{}".to_string()),
                    },
                    transition.status,
                )
                .await?;
            if update == crate::TerminalStatusUpdate::Unchanged {
                let request = self.read_request_for_caller(caller_id, request_id).await?;
                return Ok(RequestTerminalResponse {
                    latest_event_cursor: request.latest_event_cursor,
                    request: request.to_protocol(),
                });
            }
        }
        let request = self.read_request_for_caller(caller_id, request_id).await?;
        Ok(RequestTerminalResponse {
            latest_event_cursor: request.latest_event_cursor,
            request: request.to_protocol(),
        })
    }

    pub async fn terminal(
        &mut self,
        caller_id: &str,
        params: RequestTerminalParams,
    ) -> Result<RequestTerminalResponse, CloudStateError> {
        self.terminal_signal(
            caller_id,
            &params.request_id,
            params.signal,
            params.payload_inline,
        )
        .await
    }

    pub async fn terminal_for_turn_id(
        &mut self,
        turn_id: &str,
        signal: TerminalSignal,
        payload_inline: Option<String>,
    ) -> Result<RequestTerminalResponse, CloudStateError> {
        let request = self.store.read_request_by_turn_id(turn_id).await?;
        self.terminal_signal(
            &request.caller_id,
            &request.request_id,
            signal,
            payload_inline,
        )
        .await
    }

    pub async fn append_event_for_turn_id(
        &mut self,
        turn_id: &str,
        event_type: String,
        payload_inline: String,
    ) -> Result<RequestEvent, CloudStateError> {
        let request = self.store.read_request_by_turn_id(turn_id).await?;
        let event = self
            .store
            .append_request_event(EventAppendParams {
                request_id: request.request_id,
                event_type,
                payload_inline,
            })
            .await?;
        Ok(event_record_into_protocol(event))
    }

    pub async fn events_list(
        &self,
        caller_id: &str,
        params: RequestEventsListParams,
    ) -> Result<RequestEventsListResponse, CloudStateError> {
        self.read_request_for_caller(caller_id, &params.request_id)
            .await?;
        let page = self
            .store
            .list_request_events(
                &params.request_id,
                params.cursor,
                request_events_list_limit(params.limit),
            )
            .await?;
        Ok(page.into_protocol())
    }

    pub async fn persist_request_context(
        &mut self,
        caller_id: &str,
        config_snapshot: ConfigSnapshotRecord,
        state_metadata: StateMetadataRecord,
    ) -> Result<(), CloudStateError> {
        let request = self
            .read_request_for_caller(caller_id, &config_snapshot.request_id)
            .await?;
        if request.thread_id != config_snapshot.thread_id
            || request.thread_id != state_metadata.thread_id
        {
            return Err(CloudStateError::ThreadMismatch);
        }
        self.store.persist_config_snapshot(config_snapshot).await?;
        self.store.upsert_state_metadata(state_metadata).await
    }

    fn resolve_thread_id(
        &mut self,
        thread_id: Option<String>,
        create_thread: Option<bool>,
    ) -> String {
        if let Some(thread_id) = thread_id {
            return thread_id;
        }
        if create_thread.unwrap_or(false) {
            self.next_thread_sequence += 1;
            return format!("thread-{}", self.next_thread_sequence);
        }
        self.next_thread_sequence += 1;
        format!("thread-{}", self.next_thread_sequence)
    }

    async fn read_request_for_caller(
        &self,
        caller_id: &str,
        request_id: &str,
    ) -> Result<crate::RequestRecord, CloudStateError> {
        let request = self.store.read_request(request_id).await?;
        if request.caller_id != caller_id {
            return Err(CloudStateError::RequestNotFound);
        }
        Ok(request)
    }

    async fn ensure_caller_can_access_thread(
        &self,
        caller_id: &str,
        thread_id: &str,
    ) -> Result<(), CloudStateError> {
        if !self
            .store
            .caller_can_access_thread(caller_id, thread_id)
            .await?
        {
            return Err(CloudStateError::RequestNotFound);
        }
        Ok(())
    }
}

fn request_events_list_limit(limit: Option<u32>) -> usize {
    limit
        .map(|limit| limit as usize)
        .unwrap_or(REQUEST_EVENTS_LIST_DEFAULT_LIMIT)
        .clamp(1, REQUEST_EVENTS_LIST_MAX_LIMIT)
}

fn thread_items_list_limit(limit: Option<u32>) -> usize {
    limit
        .map(|limit| limit as usize)
        .unwrap_or(THREAD_ITEMS_LIST_DEFAULT_LIMIT)
        .clamp(1, THREAD_ITEMS_LIST_MAX_LIMIT)
}

#[derive(Debug)]
pub enum CloudRequestServiceRuntime {
    InMemory(Box<CloudRequestService<InMemoryCloudStateStore>>),
    Mysql(Box<CloudRequestService<MysqlCloudStateStore>>),
}

impl Default for CloudRequestServiceRuntime {
    fn default() -> Self {
        Self::InMemory(Box::default())
    }
}

impl CloudRequestServiceRuntime {
    pub async fn mysql_from_database_url(database_url: &str) -> Result<Self, CloudStateError> {
        let store = MysqlCloudStateStore::from_database_url(database_url).await?;
        Ok(Self::Mysql(Box::new(CloudRequestService::new(store))))
    }

    pub async fn mysql_from_database_url_with_max_connections(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, CloudStateError> {
        let store = MysqlCloudStateStore::from_database_url_with_max_connections(
            database_url,
            max_connections,
        )
        .await?;
        Ok(Self::Mysql(Box::new(CloudRequestService::new(store))))
    }

    pub async fn run(
        &mut self,
        caller_id: &str,
        params: RequestRunParams,
    ) -> Result<RequestRunResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => service.run(caller_id, params).await,
            Self::Mysql(service) => service.run(caller_id, params).await,
        }
    }

    pub async fn read(
        &self,
        caller_id: &str,
        params: RequestReadParams,
    ) -> Result<RequestReadResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => service.read(caller_id, params).await,
            Self::Mysql(service) => service.read(caller_id, params).await,
        }
    }

    pub async fn set_turn_id(
        &mut self,
        caller_id: &str,
        request_id: &str,
        turn_id: &str,
    ) -> Result<crate::RequestRecord, CloudStateError> {
        match self {
            Self::InMemory(service) => service.set_turn_id(caller_id, request_id, turn_id).await,
            Self::Mysql(service) => service.set_turn_id(caller_id, request_id, turn_id).await,
        }
    }

    pub async fn renew_request_leases(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<bool, CloudStateError> {
        match self {
            Self::InMemory(service) => {
                service
                    .renew_request_leases(request_id, owner_instance_id)
                    .await
            }
            Self::Mysql(service) => {
                service
                    .renew_request_leases(request_id, owner_instance_id)
                    .await
            }
        }
    }

    pub async fn expire_owner_leases(&mut self) -> Result<usize, CloudStateError> {
        match self {
            Self::InMemory(service) => service.expire_owner_leases().await,
            Self::Mysql(service) => service.expire_owner_leases().await,
        }
    }

    pub async fn append_thread_items_with_lease(
        &mut self,
        caller_id: &str,
        params: AppendThreadItemsWithLeaseParams,
    ) -> Result<AppendThreadItemsWithLeaseResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => {
                service
                    .append_thread_items_with_lease(caller_id, params)
                    .await
            }
            Self::Mysql(service) => {
                service
                    .append_thread_items_with_lease(caller_id, params)
                    .await
            }
        }
    }

    pub async fn thread_items_list(
        &self,
        caller_id: &str,
        params: ThreadItemsListParams,
    ) -> Result<ThreadItemsListResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => service.thread_items_list(caller_id, params).await,
            Self::Mysql(service) => service.thread_items_list(caller_id, params).await,
        }
    }

    pub async fn cancel(
        &mut self,
        caller_id: &str,
        params: RequestCancelParams,
    ) -> Result<RequestCancelResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => service.cancel(caller_id, params).await,
            Self::Mysql(service) => service.cancel(caller_id, params).await,
        }
    }

    pub async fn terminal_signal(
        &mut self,
        caller_id: &str,
        request_id: &str,
        signal: TerminalSignal,
        payload_inline: Option<String>,
    ) -> Result<RequestTerminalResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => {
                service
                    .terminal_signal(caller_id, request_id, signal, payload_inline)
                    .await
            }
            Self::Mysql(service) => {
                service
                    .terminal_signal(caller_id, request_id, signal, payload_inline)
                    .await
            }
        }
    }

    pub async fn terminal(
        &mut self,
        caller_id: &str,
        params: RequestTerminalParams,
    ) -> Result<RequestTerminalResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => service.terminal(caller_id, params).await,
            Self::Mysql(service) => service.terminal(caller_id, params).await,
        }
    }

    pub async fn terminal_for_turn_id(
        &mut self,
        turn_id: &str,
        signal: TerminalSignal,
        payload_inline: Option<String>,
    ) -> Result<RequestTerminalResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => {
                service
                    .terminal_for_turn_id(turn_id, signal, payload_inline)
                    .await
            }
            Self::Mysql(service) => {
                service
                    .terminal_for_turn_id(turn_id, signal, payload_inline)
                    .await
            }
        }
    }

    pub async fn append_event_for_turn_id(
        &mut self,
        turn_id: &str,
        event_type: String,
        payload_inline: String,
    ) -> Result<RequestEvent, CloudStateError> {
        match self {
            Self::InMemory(service) => {
                service
                    .append_event_for_turn_id(turn_id, event_type, payload_inline)
                    .await
            }
            Self::Mysql(service) => {
                service
                    .append_event_for_turn_id(turn_id, event_type, payload_inline)
                    .await
            }
        }
    }

    pub async fn events_list(
        &self,
        caller_id: &str,
        params: RequestEventsListParams,
    ) -> Result<RequestEventsListResponse, CloudStateError> {
        match self {
            Self::InMemory(service) => service.events_list(caller_id, params).await,
            Self::Mysql(service) => service.events_list(caller_id, params).await,
        }
    }

    pub async fn persist_request_context(
        &mut self,
        caller_id: &str,
        config_snapshot: ConfigSnapshotRecord,
        state_metadata: StateMetadataRecord,
    ) -> Result<(), CloudStateError> {
        match self {
            Self::InMemory(service) => {
                service
                    .persist_request_context(caller_id, config_snapshot, state_metadata)
                    .await
            }
            Self::Mysql(service) => {
                service
                    .persist_request_context(caller_id, config_snapshot, state_metadata)
                    .await
            }
        }
    }
}

impl EventsPage {
    fn into_protocol(self) -> RequestEventsListResponse {
        RequestEventsListResponse {
            data: self
                .data
                .into_iter()
                .map(event_record_into_protocol)
                .collect(),
            next_cursor: self.next_cursor,
        }
    }
}

fn event_record_into_protocol(event: crate::EventRecord) -> RequestEvent {
    RequestEvent {
        request_id: event.request_id,
        sequence: event.sequence,
        event_type: event.event_type,
        payload_inline: event.payload_inline,
    }
}
