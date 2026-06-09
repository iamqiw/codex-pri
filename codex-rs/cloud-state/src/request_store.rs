use std::collections::HashMap;
use std::future::Future;

use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseResponse;
use codex_cloud_wrapper_protocol::RequestRecord as ProtocolRequestRecord;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::ThreadItemRecord;
use codex_cloud_wrapper_protocol::WriterLeaseRecord;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequestParams {
    pub caller_id: String,
    pub thread_id: String,
    pub idempotency_key: String,
    pub input_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    pub request_id: String,
    pub caller_id: String,
    pub thread_id: String,
    pub idempotency_key: String,
    pub input_hash: String,
    pub turn_id: Option<String>,
    pub status: RequestStatus,
    pub latest_event_cursor: u64,
}

impl RequestRecord {
    pub fn to_protocol(&self) -> ProtocolRequestRecord {
        ProtocolRequestRecord {
            request_id: self.request_id.clone(),
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
            status: self.status,
            latest_event_cursor: self.latest_event_cursor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseAppendParams {
    pub request_id: String,
    pub lease_id: String,
    pub writer_owner_token: String,
    pub fencing_token: u64,
    pub append_idempotency_key: String,
    pub expected_thread_version: Option<u64>,
    pub payload_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAppendParams {
    pub request_id: String,
    pub event_type: String,
    pub payload_inline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub request_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub payload_inline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsPage {
    pub data: Vec<EventRecord>,
    pub next_cursor: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSnapshotRecord {
    pub request_id: String,
    pub thread_id: String,
    pub config_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMetadataRecord {
    pub thread_id: String,
    pub kind: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppendResultRecord {
    request_id: String,
    response: AppendThreadItemsWithLeaseResponse,
    payload_refs_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStatusUpdate {
    Changed,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CloudStateError {
    #[error("idempotency key was reused with different input")]
    IdempotencyConflict,

    #[error("request was not found")]
    RequestNotFound,

    #[error("request lost its writer lease")]
    LeaseLost,

    #[error("request is not queued and cannot acquire a writer lease")]
    RequestNotQueued,

    #[error("request event payload must be valid JSON")]
    InvalidEventPayload,

    #[error("append thread_id does not match request thread_id")]
    ThreadMismatch,

    #[error("expected thread version does not match current thread version")]
    ThreadVersionConflict,

    #[error("append must include at least one thread item")]
    EmptyAppend,

    #[error("cloud state storage error: {0}")]
    Storage(String),
}

/// Persistence boundary for cloud wrapper request state.
///
/// Implementations must preserve request idempotency, per-thread writer lease
/// fencing, terminal status updates, append deduplication for identical payload
/// references, and request event cursor semantics. Durable implementations are
/// expected to enforce these invariants transactionally.
pub trait CloudStateStore {
    fn create_request(
        &mut self,
        params: CreateRequestParams,
    ) -> impl Future<Output = Result<RequestRecord, CloudStateError>> + Send;

    fn read_request<'a>(
        &'a self,
        request_id: &'a str,
    ) -> impl Future<Output = Result<RequestRecord, CloudStateError>> + Send + 'a;

    fn read_request_by_turn_id<'a>(
        &'a self,
        turn_id: &'a str,
    ) -> impl Future<Output = Result<RequestRecord, CloudStateError>> + Send + 'a;

    fn set_request_turn_id<'a>(
        &'a mut self,
        request_id: &'a str,
        turn_id: &'a str,
    ) -> impl Future<Output = Result<RequestRecord, CloudStateError>> + Send + 'a;

    fn caller_can_access_thread<'a>(
        &'a self,
        caller_id: &'a str,
        thread_id: &'a str,
    ) -> impl Future<Output = Result<bool, CloudStateError>> + Send + 'a;

    fn acquire_thread_writer_lease(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> impl Future<Output = Result<LeaseAcquireOutcome, CloudStateError>> + Send;

    fn read_thread_writer_lease<'a>(
        &'a self,
        request_id: &'a str,
    ) -> impl Future<Output = Result<Option<WriterLeaseRecord>, CloudStateError>> + Send + 'a;

    fn renew_request_leases<'a>(
        &'a mut self,
        request_id: &'a str,
        owner_instance_id: &'a str,
    ) -> impl Future<Output = Result<bool, CloudStateError>> + Send + 'a;

    fn expire_owner_leases(
        &mut self,
    ) -> impl Future<Output = Result<usize, CloudStateError>> + Send;

    fn append_thread_items_with_lease(
        &mut self,
        params: LeaseAppendParams,
    ) -> impl Future<Output = Result<AppendThreadItemsWithLeaseResponse, CloudStateError>> + Send;

    fn list_thread_items<'a>(
        &'a self,
        thread_id: &'a str,
        cursor: Option<u64>,
        limit: usize,
    ) -> impl Future<Output = Result<(Vec<ThreadItemRecord>, Option<u64>), CloudStateError>> + Send + 'a;

    fn mark_request_terminal(
        &mut self,
        request_id: &str,
        status: RequestStatus,
    ) -> impl Future<Output = Result<TerminalStatusUpdate, CloudStateError>> + Send;

    fn mark_request_terminal_with_event(
        &mut self,
        params: EventAppendParams,
        status: RequestStatus,
    ) -> impl Future<Output = Result<TerminalStatusUpdate, CloudStateError>> + Send;

    fn append_request_event(
        &mut self,
        params: EventAppendParams,
    ) -> impl Future<Output = Result<EventRecord, CloudStateError>> + Send;

    fn append_request_event_if_latest_cursor(
        &mut self,
        params: EventAppendParams,
        expected_latest_event_cursor: u64,
    ) -> impl Future<Output = Result<Option<EventRecord>, CloudStateError>> + Send;

    fn list_request_events(
        &self,
        request_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> impl Future<Output = Result<EventsPage, CloudStateError>> + Send;

    fn persist_config_snapshot(
        &mut self,
        record: ConfigSnapshotRecord,
    ) -> impl Future<Output = Result<(), CloudStateError>> + Send;

    fn read_config_snapshot<'a>(
        &'a self,
        request_id: &'a str,
    ) -> impl Future<Output = Result<ConfigSnapshotRecord, CloudStateError>> + Send + 'a;

    fn upsert_state_metadata(
        &mut self,
        record: StateMetadataRecord,
    ) -> impl Future<Output = Result<(), CloudStateError>> + Send;

    fn read_state_metadata<'a>(
        &'a self,
        thread_id: &'a str,
        kind: &'a str,
    ) -> impl Future<Output = Result<StateMetadataRecord, CloudStateError>> + Send + 'a;
}

#[derive(Debug, Default)]
pub struct InMemoryCloudStateStore {
    requests_by_id: HashMap<String, RequestRecord>,
    requests_by_idempotency_key: HashMap<(String, String, String), RequestRecord>,
    held_leases_by_thread: HashMap<String, ThreadWriterLeaseRecord>,
    append_responses_by_key: HashMap<(String, String), AppendResultRecord>,
    events_by_request: HashMap<String, Vec<EventRecord>>,
    thread_items_by_thread: HashMap<String, Vec<ThreadItemRecord>>,
    config_snapshots_by_request: HashMap<String, ConfigSnapshotRecord>,
    state_metadata_by_thread_kind: HashMap<(String, String), StateMetadataRecord>,
    thread_versions: HashMap<String, u64>,
    next_request_sequence: u64,
    next_lease_sequence: u64,
    next_fencing_token: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquireOutcome {
    Acquired {
        lease_id: String,
        fencing_token: u64,
    },
    Queued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThreadWriterLeaseRecord {
    lease_id: String,
    request_id: String,
    owner_instance_id: String,
    fencing_token: u64,
}

impl InMemoryCloudStateStore {
    pub fn create_request(
        &mut self,
        params: CreateRequestParams,
    ) -> Result<RequestRecord, CloudStateError> {
        let key = (
            params.caller_id.clone(),
            params.thread_id.clone(),
            params.idempotency_key.clone(),
        );

        if let Some(existing) = self.requests_by_idempotency_key.get(&key) {
            if existing.input_hash != params.input_hash {
                return Err(CloudStateError::IdempotencyConflict);
            }
            return Ok(existing.clone());
        }

        self.next_request_sequence += 1;
        let record = RequestRecord {
            request_id: format!("request-{}", self.next_request_sequence),
            caller_id: params.caller_id,
            thread_id: params.thread_id,
            idempotency_key: params.idempotency_key,
            input_hash: params.input_hash,
            turn_id: None,
            status: RequestStatus::Queued,
            latest_event_cursor: 0,
        };

        self.requests_by_idempotency_key.insert(key, record.clone());
        self.requests_by_id
            .insert(record.request_id.clone(), record.clone());

        Ok(record)
    }

    pub fn read_request(&self, request_id: &str) -> Result<RequestRecord, CloudStateError> {
        self.requests_by_id
            .get(request_id)
            .cloned()
            .ok_or(CloudStateError::RequestNotFound)
    }

    pub fn read_request_by_turn_id(&self, turn_id: &str) -> Result<RequestRecord, CloudStateError> {
        self.requests_by_id
            .values()
            .find(|request| request.turn_id.as_deref() == Some(turn_id))
            .cloned()
            .ok_or(CloudStateError::RequestNotFound)
    }

    pub fn set_request_turn_id(
        &mut self,
        request_id: &str,
        turn_id: &str,
    ) -> Result<RequestRecord, CloudStateError> {
        let mut record = self.read_request(request_id)?;
        record.turn_id = Some(turn_id.to_string());
        self.requests_by_id
            .insert(request_id.to_string(), record.clone());
        let key = (
            record.caller_id.clone(),
            record.thread_id.clone(),
            record.idempotency_key.clone(),
        );
        self.requests_by_idempotency_key.insert(key, record.clone());
        Ok(record)
    }

    pub fn caller_can_access_thread(
        &self,
        caller_id: &str,
        thread_id: &str,
    ) -> Result<bool, CloudStateError> {
        Ok(self
            .requests_by_id
            .values()
            .any(|request| request.caller_id == caller_id && request.thread_id == thread_id))
    }

    pub fn acquire_thread_writer_lease(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<LeaseAcquireOutcome, CloudStateError> {
        let request = self.read_request(request_id)?;

        if self.held_leases_by_thread.contains_key(&request.thread_id) {
            return Ok(LeaseAcquireOutcome::Queued);
        }
        if request.status != RequestStatus::Queued {
            return Err(CloudStateError::RequestNotQueued);
        }

        self.next_lease_sequence += 1;
        self.next_fencing_token += 1;
        let lease = ThreadWriterLeaseRecord {
            lease_id: format!("lease-{}", self.next_lease_sequence),
            request_id: request.request_id.clone(),
            owner_instance_id: owner_instance_id.to_string(),
            fencing_token: self.next_fencing_token,
        };

        self.held_leases_by_thread
            .insert(request.thread_id.clone(), lease.clone());
        self.set_request_status(&request.request_id, RequestStatus::Running)?;

        Ok(LeaseAcquireOutcome::Acquired {
            lease_id: lease.lease_id,
            fencing_token: lease.fencing_token,
        })
    }

    pub fn read_thread_writer_lease(
        &self,
        request_id: &str,
    ) -> Result<Option<WriterLeaseRecord>, CloudStateError> {
        let request = self.read_request(request_id)?;
        Ok(self
            .held_leases_by_thread
            .get(&request.thread_id)
            .filter(|lease| lease.request_id == request_id)
            .map(|lease| WriterLeaseRecord {
                lease_id: lease.lease_id.clone(),
                writer_owner_token: lease.owner_instance_id.clone(),
                fencing_token: lease.fencing_token,
            }))
    }

    pub fn renew_request_leases(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<bool, CloudStateError> {
        let request = self.read_request(request_id)?;
        if request.status.is_terminal() {
            return Ok(false);
        }
        let active = self
            .held_leases_by_thread
            .get(&request.thread_id)
            .is_some_and(|lease| {
                lease.request_id == request_id && lease.owner_instance_id == owner_instance_id
            });
        if active {
            return Ok(true);
        }
        self.mark_request_terminal_with_event(
            EventAppendParams {
                request_id: request_id.to_string(),
                event_type: "request/lease_lost".to_string(),
                payload_inline: r#"{"reason":"thread_writer_lease_lost"}"#.to_string(),
            },
            RequestStatus::LeaseLost,
        )?;
        Ok(false)
    }

    pub fn expire_owner_leases(&mut self) -> Result<usize, CloudStateError> {
        Ok(0)
    }

    pub fn append_thread_items_with_lease(
        &mut self,
        params: LeaseAppendParams,
    ) -> Result<AppendThreadItemsWithLeaseResponse, CloudStateError> {
        if params.payload_refs.is_empty() {
            return Err(CloudStateError::EmptyAppend);
        }

        let request = self.read_request(&params.request_id)?;
        let append_key = (
            request.thread_id.clone(),
            params.append_idempotency_key.clone(),
        );
        let payload_refs_fingerprint = append_payload_refs_fingerprint(&params.payload_refs)?;
        let valid_lease = self
            .held_leases_by_thread
            .get(&request.thread_id)
            .is_some_and(|lease| {
                lease.request_id == params.request_id
                    && lease.lease_id == params.lease_id
                    && lease.owner_instance_id == params.writer_owner_token
                    && lease.fencing_token == params.fencing_token
            });

        if !valid_lease {
            if self
                .held_leases_by_thread
                .get(&request.thread_id)
                .is_some_and(|lease| lease.request_id == params.request_id)
            {
                self.held_leases_by_thread.remove(&request.thread_id);
                self.set_request_status(&params.request_id, RequestStatus::LeaseLost)?;
            }
            return Err(CloudStateError::LeaseLost);
        }

        if let Some(existing) = self.append_responses_by_key.get(&append_key) {
            if existing.request_id != params.request_id {
                return Err(CloudStateError::IdempotencyConflict);
            }
            if !append_payload_refs_match(&existing.payload_refs_fingerprint, &params.payload_refs)
            {
                return Err(CloudStateError::IdempotencyConflict);
            }
            return Ok(AppendThreadItemsWithLeaseResponse {
                deduplicated: true,
                ..existing.response.clone()
            });
        }

        let item_count = params.payload_refs.len() as u64;
        let current_version = self
            .thread_versions
            .get(&request.thread_id)
            .copied()
            .unwrap_or(0);
        if params
            .expected_thread_version
            .is_some_and(|expected| expected != current_version)
        {
            return Err(CloudStateError::ThreadVersionConflict);
        }
        let next_version = current_version + item_count;
        let thread_id = request.thread_id;
        self.thread_versions.insert(thread_id.clone(), next_version);
        self.thread_items_by_thread
            .entry(thread_id.clone())
            .or_default()
            .extend(
                params
                    .payload_refs
                    .into_iter()
                    .enumerate()
                    .map(|(index, payload_ref)| ThreadItemRecord {
                        thread_id: thread_id.clone(),
                        sequence: current_version + index as u64 + 1,
                        payload_ref,
                    }),
            );

        let response = AppendThreadItemsWithLeaseResponse {
            thread_version: next_version,
            first_append_sequence: current_version + 1,
            last_append_sequence: next_version,
            deduplicated: false,
        };
        self.append_responses_by_key.insert(
            append_key,
            AppendResultRecord {
                request_id: params.request_id,
                response: response.clone(),
                payload_refs_fingerprint,
            },
        );

        Ok(response)
    }

    pub fn list_thread_items(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<ThreadItemRecord>, Option<u64>), CloudStateError> {
        let after = cursor.unwrap_or(0);
        let mut data = self
            .thread_items_by_thread
            .get(thread_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item.sequence > after)
            .take(limit + 1)
            .collect::<Vec<_>>();
        let has_more = data.len() > limit;
        data.truncate(limit);
        let next_cursor = if has_more {
            data.last().map(|item| item.sequence)
        } else {
            None
        };

        Ok((data, next_cursor))
    }

    pub fn mark_request_terminal(
        &mut self,
        request_id: &str,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        debug_assert!(status.is_terminal());
        let request = self.read_request(request_id)?;
        let update = self.set_request_status(request_id, status)?;
        if self
            .held_leases_by_thread
            .get(&request.thread_id)
            .is_some_and(|lease| lease.request_id == request_id)
        {
            self.held_leases_by_thread.remove(&request.thread_id);
        }
        Ok(update)
    }

    pub fn mark_request_terminal_with_event(
        &mut self,
        params: EventAppendParams,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        debug_assert!(status.is_terminal());
        validate_event_payload_inline(&params.payload_inline)?;
        let request = self.read_request(&params.request_id)?;
        let update = self.set_request_status(&params.request_id, status)?;
        if update == TerminalStatusUpdate::Unchanged {
            return Ok(TerminalStatusUpdate::Unchanged);
        }
        if self
            .held_leases_by_thread
            .get(&request.thread_id)
            .is_some_and(|lease| lease.request_id == params.request_id)
        {
            self.held_leases_by_thread.remove(&request.thread_id);
        }
        self.append_request_event(params)?;
        Ok(TerminalStatusUpdate::Changed)
    }

    pub fn append_request_event(
        &mut self,
        params: EventAppendParams,
    ) -> Result<EventRecord, CloudStateError> {
        validate_event_payload_inline(&params.payload_inline)?;
        self.read_request(&params.request_id)?;
        let sequence = self
            .events_by_request
            .get(&params.request_id)
            .map_or(1, |events| events.len() as u64 + 1);
        let event = EventRecord {
            request_id: params.request_id.clone(),
            sequence,
            event_type: params.event_type,
            payload_inline: params.payload_inline,
        };

        self.events_by_request
            .entry(params.request_id.clone())
            .or_default()
            .push(event.clone());
        self.set_latest_event_cursor(&params.request_id, sequence)?;

        Ok(event)
    }

    pub fn append_request_event_if_latest_cursor(
        &mut self,
        params: EventAppendParams,
        expected_latest_event_cursor: u64,
    ) -> Result<Option<EventRecord>, CloudStateError> {
        validate_event_payload_inline(&params.payload_inline)?;
        let request = self.read_request(&params.request_id)?;
        if request.latest_event_cursor != expected_latest_event_cursor {
            return Ok(None);
        }
        self.append_request_event(params).map(Some)
    }

    pub fn list_request_events(
        &self,
        request_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<EventsPage, CloudStateError> {
        self.read_request(request_id)?;
        let after = cursor.unwrap_or(0);
        let events = self
            .events_by_request
            .get(request_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let data = events
            .iter()
            .filter(|event| event.sequence > after)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_cursor = if data.len() == limit
            && events
                .iter()
                .any(|event| event.sequence > data.last().map_or(after, |event| event.sequence))
        {
            data.last().map(|event| event.sequence)
        } else {
            None
        };

        Ok(EventsPage { data, next_cursor })
    }

    pub fn persist_config_snapshot(
        &mut self,
        record: ConfigSnapshotRecord,
    ) -> Result<(), CloudStateError> {
        let request = self.read_request(&record.request_id)?;
        if request.thread_id != record.thread_id {
            return Err(CloudStateError::ThreadMismatch);
        }
        validate_json_payload(&record.config_json)?;
        self.config_snapshots_by_request
            .insert(record.request_id.clone(), record);
        Ok(())
    }

    pub fn read_config_snapshot(
        &self,
        request_id: &str,
    ) -> Result<ConfigSnapshotRecord, CloudStateError> {
        self.read_request(request_id)?;
        self.config_snapshots_by_request
            .get(request_id)
            .cloned()
            .ok_or(CloudStateError::RequestNotFound)
    }

    pub fn upsert_state_metadata(
        &mut self,
        record: StateMetadataRecord,
    ) -> Result<(), CloudStateError> {
        validate_json_payload(&record.payload_json)?;
        self.state_metadata_by_thread_kind
            .insert((record.thread_id.clone(), record.kind.clone()), record);
        Ok(())
    }

    pub fn read_state_metadata(
        &self,
        thread_id: &str,
        kind: &str,
    ) -> Result<StateMetadataRecord, CloudStateError> {
        self.state_metadata_by_thread_kind
            .get(&(thread_id.to_string(), kind.to_string()))
            .cloned()
            .ok_or(CloudStateError::RequestNotFound)
    }

    fn set_request_status(
        &mut self,
        request_id: &str,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        let request = self
            .requests_by_id
            .get_mut(request_id)
            .ok_or(CloudStateError::RequestNotFound)?;
        if request.status.is_terminal() && request.status != status {
            return Ok(TerminalStatusUpdate::Unchanged);
        }
        if request.status == status {
            return Ok(TerminalStatusUpdate::Unchanged);
        }
        request.status = status;
        let key = (
            request.caller_id.clone(),
            request.thread_id.clone(),
            request.idempotency_key.clone(),
        );
        self.requests_by_idempotency_key
            .insert(key, request.clone());
        Ok(TerminalStatusUpdate::Changed)
    }

    fn set_latest_event_cursor(
        &mut self,
        request_id: &str,
        latest_event_cursor: u64,
    ) -> Result<(), CloudStateError> {
        let request = self
            .requests_by_id
            .get_mut(request_id)
            .ok_or(CloudStateError::RequestNotFound)?;
        request.latest_event_cursor = latest_event_cursor;
        let key = (
            request.caller_id.clone(),
            request.thread_id.clone(),
            request.idempotency_key.clone(),
        );
        self.requests_by_idempotency_key
            .insert(key, request.clone());
        Ok(())
    }
}

pub(crate) fn append_payload_refs_fingerprint(
    payload_refs: &[String],
) -> Result<String, CloudStateError> {
    serde_json::to_string(payload_refs).map_err(|err| {
        CloudStateError::Storage(format!("failed to serialize append payload refs: {err}"))
    })
}

pub(crate) fn append_payload_refs_match(fingerprint: &str, payload_refs: &[String]) -> bool {
    serde_json::from_str::<Vec<String>>(fingerprint).is_ok_and(|existing| existing == payload_refs)
}

pub(crate) fn validate_event_payload_inline(payload_inline: &str) -> Result<(), CloudStateError> {
    validate_json_payload(payload_inline).map_err(|_| CloudStateError::InvalidEventPayload)
}

fn validate_json_payload(payload_inline: &str) -> Result<(), CloudStateError> {
    serde_json::from_str::<serde_json::Value>(payload_inline)
        .map(|_| ())
        .map_err(|_| CloudStateError::Storage("payload must be valid JSON".to_string()))
}

impl CloudStateStore for InMemoryCloudStateStore {
    async fn create_request(
        &mut self,
        params: CreateRequestParams,
    ) -> Result<RequestRecord, CloudStateError> {
        InMemoryCloudStateStore::create_request(self, params)
    }

    async fn read_request(&self, request_id: &str) -> Result<RequestRecord, CloudStateError> {
        InMemoryCloudStateStore::read_request(self, request_id)
    }

    async fn read_request_by_turn_id(
        &self,
        turn_id: &str,
    ) -> Result<RequestRecord, CloudStateError> {
        InMemoryCloudStateStore::read_request_by_turn_id(self, turn_id)
    }

    async fn set_request_turn_id(
        &mut self,
        request_id: &str,
        turn_id: &str,
    ) -> Result<RequestRecord, CloudStateError> {
        InMemoryCloudStateStore::set_request_turn_id(self, request_id, turn_id)
    }

    async fn caller_can_access_thread(
        &self,
        caller_id: &str,
        thread_id: &str,
    ) -> Result<bool, CloudStateError> {
        InMemoryCloudStateStore::caller_can_access_thread(self, caller_id, thread_id)
    }

    async fn acquire_thread_writer_lease(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<LeaseAcquireOutcome, CloudStateError> {
        InMemoryCloudStateStore::acquire_thread_writer_lease(self, request_id, owner_instance_id)
    }

    async fn read_thread_writer_lease(
        &self,
        request_id: &str,
    ) -> Result<Option<WriterLeaseRecord>, CloudStateError> {
        InMemoryCloudStateStore::read_thread_writer_lease(self, request_id)
    }

    async fn renew_request_leases(
        &mut self,
        request_id: &str,
        owner_instance_id: &str,
    ) -> Result<bool, CloudStateError> {
        InMemoryCloudStateStore::renew_request_leases(self, request_id, owner_instance_id)
    }

    async fn expire_owner_leases(&mut self) -> Result<usize, CloudStateError> {
        InMemoryCloudStateStore::expire_owner_leases(self)
    }

    async fn append_thread_items_with_lease(
        &mut self,
        params: LeaseAppendParams,
    ) -> Result<AppendThreadItemsWithLeaseResponse, CloudStateError> {
        InMemoryCloudStateStore::append_thread_items_with_lease(self, params)
    }

    async fn list_thread_items(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<ThreadItemRecord>, Option<u64>), CloudStateError> {
        InMemoryCloudStateStore::list_thread_items(self, thread_id, cursor, limit)
    }

    async fn mark_request_terminal(
        &mut self,
        request_id: &str,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        InMemoryCloudStateStore::mark_request_terminal(self, request_id, status)
    }

    async fn mark_request_terminal_with_event(
        &mut self,
        params: EventAppendParams,
        status: RequestStatus,
    ) -> Result<TerminalStatusUpdate, CloudStateError> {
        InMemoryCloudStateStore::mark_request_terminal_with_event(self, params, status)
    }

    async fn append_request_event(
        &mut self,
        params: EventAppendParams,
    ) -> Result<EventRecord, CloudStateError> {
        InMemoryCloudStateStore::append_request_event(self, params)
    }

    async fn append_request_event_if_latest_cursor(
        &mut self,
        params: EventAppendParams,
        expected_latest_event_cursor: u64,
    ) -> Result<Option<EventRecord>, CloudStateError> {
        InMemoryCloudStateStore::append_request_event_if_latest_cursor(
            self,
            params,
            expected_latest_event_cursor,
        )
    }

    async fn list_request_events(
        &self,
        request_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<EventsPage, CloudStateError> {
        InMemoryCloudStateStore::list_request_events(self, request_id, cursor, limit)
    }

    async fn persist_config_snapshot(
        &mut self,
        record: ConfigSnapshotRecord,
    ) -> Result<(), CloudStateError> {
        InMemoryCloudStateStore::persist_config_snapshot(self, record)
    }

    async fn read_config_snapshot(
        &self,
        request_id: &str,
    ) -> Result<ConfigSnapshotRecord, CloudStateError> {
        InMemoryCloudStateStore::read_config_snapshot(self, request_id)
    }

    async fn upsert_state_metadata(
        &mut self,
        record: StateMetadataRecord,
    ) -> Result<(), CloudStateError> {
        InMemoryCloudStateStore::upsert_state_metadata(self, record)
    }

    async fn read_state_metadata(
        &self,
        thread_id: &str,
        kind: &str,
    ) -> Result<StateMetadataRecord, CloudStateError> {
        InMemoryCloudStateStore::read_state_metadata(self, thread_id, kind)
    }
}
