use codex_app_server_protocol::JSONRPCErrorError;
use codex_cloud_state::CloudRequestServiceRuntime;
use codex_cloud_state::CloudStateError;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseParams;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseResponse;
use codex_cloud_wrapper_protocol::RequestCancelParams;
use codex_cloud_wrapper_protocol::RequestCancelResponse;
use codex_cloud_wrapper_protocol::RequestEventsListParams;
use codex_cloud_wrapper_protocol::RequestEventsListResponse;
use codex_cloud_wrapper_protocol::RequestReadParams;
use codex_cloud_wrapper_protocol::RequestReadResponse;
use codex_cloud_wrapper_protocol::RequestRunParams;
use codex_cloud_wrapper_protocol::RequestRunResponse;
use codex_cloud_wrapper_protocol::RequestTerminalParams;
use codex_cloud_wrapper_protocol::RequestTerminalResponse;
use codex_cloud_wrapper_protocol::ThreadItemsListParams;
use codex_cloud_wrapper_protocol::ThreadItemsListResponse;
use codex_core::config::CloudRuntimeStateStore;
use codex_core::config::Config;
use futures::lock::Mutex;

use crate::error_code::internal_error;
use crate::error_code::invalid_params;

const DEFAULT_MYSQL_URL_ENV_VAR: &str = "CODEX_CLOUD_STATE_MYSQL_URL";

#[derive(Debug)]
pub(crate) struct CloudWrapperRequestProcessor {
    request_service: Mutex<CloudWrapperRequestServiceState>,
}

impl CloudWrapperRequestProcessor {
    pub(crate) fn new(config: &Config) -> Self {
        let state = match config.cloud_runtime.state_store {
            CloudRuntimeStateStore::InMemory => {
                CloudWrapperRequestServiceState::Ready(CloudRequestServiceRuntime::default())
            }
            CloudRuntimeStateStore::Mysql => CloudWrapperRequestServiceState::UninitializedMysql {
                database_url_env_var: config
                    .cloud_runtime
                    .mysql_url_env_var
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MYSQL_URL_ENV_VAR.to_string()),
                max_connections: config.cloud_runtime.mysql_max_connections,
            },
        };
        Self {
            request_service: Mutex::new(state),
        }
    }

    pub(crate) async fn request_run(
        &self,
        caller_id: &str,
        params: RequestRunParams,
    ) -> Result<RequestRunResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .run(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }

    pub(crate) async fn request_read(
        &self,
        caller_id: &str,
        params: RequestReadParams,
    ) -> Result<RequestReadResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .read(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }

    pub(crate) async fn thread_append_with_lease(
        &self,
        caller_id: &str,
        params: AppendThreadItemsWithLeaseParams,
    ) -> Result<AppendThreadItemsWithLeaseResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .append_thread_items_with_lease(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }

    pub(crate) async fn thread_items_list(
        &self,
        caller_id: &str,
        params: ThreadItemsListParams,
    ) -> Result<ThreadItemsListResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .thread_items_list(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }

    pub(crate) async fn request_cancel(
        &self,
        caller_id: &str,
        params: RequestCancelParams,
    ) -> Result<RequestCancelResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .cancel(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }

    pub(crate) async fn request_terminal(
        &self,
        caller_id: &str,
        params: RequestTerminalParams,
    ) -> Result<RequestTerminalResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .terminal(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }

    pub(crate) async fn request_events_list(
        &self,
        caller_id: &str,
        params: RequestEventsListParams,
    ) -> Result<RequestEventsListResponse, JSONRPCErrorError> {
        let mut request_service = self.request_service.lock().await;
        let request_service = request_service.service_mut().await?;
        request_service
            .events_list(caller_id, params)
            .await
            .map_err(cloud_state_error)
    }
}

impl Default for CloudWrapperRequestProcessor {
    fn default() -> Self {
        Self {
            request_service: Mutex::new(CloudWrapperRequestServiceState::Ready(
                CloudRequestServiceRuntime::default(),
            )),
        }
    }
}

#[derive(Debug)]
enum CloudWrapperRequestServiceState {
    Ready(CloudRequestServiceRuntime),
    UninitializedMysql {
        database_url_env_var: String,
        max_connections: u32,
    },
}

impl CloudWrapperRequestServiceState {
    async fn service_mut(&mut self) -> Result<&mut CloudRequestServiceRuntime, JSONRPCErrorError> {
        if let Self::UninitializedMysql {
            database_url_env_var,
            max_connections,
        } = self
        {
            let database_url = std::env::var(database_url_env_var.as_str()).map_err(|_| {
                internal_error(format!(
                    "cloud runtime MySQL state store requires {database_url_env_var} to be set"
                ))
            })?;
            *self = Self::Ready(
                CloudRequestServiceRuntime::mysql_from_database_url_with_max_connections(
                    &database_url,
                    *max_connections,
                )
                .await
                .map_err(cloud_state_error)?,
            );
        }
        match self {
            Self::Ready(service) => Ok(service),
            Self::UninitializedMysql { .. } => unreachable!("service should be initialized"),
        }
    }
}

fn cloud_state_error(err: CloudStateError) -> JSONRPCErrorError {
    match err {
        CloudStateError::IdempotencyConflict
        | CloudStateError::EmptyAppend
        | CloudStateError::InvalidEventPayload
        | CloudStateError::LeaseLost
        | CloudStateError::RequestNotQueued
        | CloudStateError::RequestNotFound
        | CloudStateError::ThreadMismatch
        | CloudStateError::ThreadVersionConflict => invalid_params(err.to_string()),
        CloudStateError::Storage(_) => internal_error(err.to_string()),
    }
}

#[cfg(test)]
#[path = "cloud_wrapper_processor_tests.rs"]
mod cloud_wrapper_processor_tests;
