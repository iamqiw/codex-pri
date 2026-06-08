use anyhow::Context;
use anyhow::Result;
use app_test_support::TestAppServer;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::to_response;
use codex_app_server::INVALID_PARAMS_ERROR_CODE;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::RequestId;
use codex_cloud_state::CloudStateStore;
use codex_cloud_state::MysqlCloudStateStore;
use codex_cloud_wrapper_protocol::AppendThreadItemsWithLeaseResponse;
use codex_cloud_wrapper_protocol::RequestCancelResponse;
use codex_cloud_wrapper_protocol::RequestEventsListResponse;
use codex_cloud_wrapper_protocol::RequestReadResponse;
use codex_cloud_wrapper_protocol::RequestRunResponse;
use codex_cloud_wrapper_protocol::RequestStatus;
use codex_cloud_wrapper_protocol::RequestTerminalResponse;
use codex_cloud_wrapper_protocol::ThreadItemRecord;
use codex_cloud_wrapper_protocol::ThreadItemsListResponse;
use pretty_assertions::assert_eq;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

const MYSQL_URL_ENV_VAR: &str = "CODEX_CLOUD_STATE_MYSQL_URL";

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_run_persists_to_mysql_across_app_server_restart() -> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    let mut restarted_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
    restarted_mcp.initialize().await?;
    let read_request_id = restarted_mcp
        .send_raw_request(
            "request/read",
            Some(serde_json::json!({
                "requestId": run_response.request.request_id,
            })),
        )
        .await?;
    let read_response = restarted_mcp
        .read_stream_until_response_message(RequestId::Integer(read_request_id))
        .await?;
    let read_response = to_response::<RequestReadResponse>(read_response)?;

    assert_eq!(read_response.request.status, RequestStatus::Running);
    assert_eq!(
        read_response.latest_event_cursor,
        run_response.request.latest_event_cursor
    );
    assert_eq!(read_response.request, run_response.request);

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_and_thread_operations_reject_different_client_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let owner_codex_home = TempDir::new()?;
    let other_codex_home = TempDir::new()?;
    let read_codex_home = TempDir::new()?;
    create_config_toml(owner_codex_home.path(), &server.uri())?;
    create_config_toml(other_codex_home.path(), &server.uri())?;
    create_config_toml(read_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(owner_codex_home.path(), &env).await?;
        initialize_as(&mut mcp, "caller-1").await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };
    let writer_lease = run_response
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    let mut other_mcp = TestAppServer::new_with_env(other_codex_home.path(), &env).await?;
    initialize_as(&mut other_mcp, "caller-2").await?;
    let read_error = request_error(
        &mut other_mcp,
        "request/read",
        serde_json::json!({
            "requestId": run_response.request.request_id.clone(),
        }),
    )
    .await?;
    let events_error = request_error(
        &mut other_mcp,
        "request/events/list",
        serde_json::json!({
            "requestId": run_response.request.request_id.clone(),
            "cursor": null,
            "limit": 10,
        }),
    )
    .await?;
    let append_error = request_error(
        &mut other_mcp,
        "thread/appendWithLease",
        serde_json::json!({
            "threadId": run_response.thread_id.clone(),
            "requestId": run_response.request.request_id.clone(),
            "leaseId": writer_lease.lease_id,
            "writerOwnerToken": writer_lease.writer_owner_token,
            "fencingToken": writer_lease.fencing_token,
            "appendIdempotencyKey": format!("append-{test_id}"),
            "expectedThreadVersion": 0,
            "items": [
                {
                    "payloadRef": format!("object://items/{test_id}"),
                }
            ],
        }),
    )
    .await?;
    let thread_items_error = request_error(
        &mut other_mcp,
        "thread/items/list",
        serde_json::json!({
            "threadId": run_response.thread_id.clone(),
            "cursor": null,
            "limit": 10,
        }),
    )
    .await?;
    let cancel_error = request_error(
        &mut other_mcp,
        "request/cancel",
        serde_json::json!({
            "requestId": run_response.request.request_id.clone(),
            "reason": "wrong-caller",
        }),
    )
    .await?;
    let terminal_error = request_error(
        &mut other_mcp,
        "request/terminal",
        serde_json::json!({
            "requestId": run_response.request.request_id.clone(),
            "signal": "completed",
            "payloadInline": null,
        }),
    )
    .await?;

    for error in [
        read_error,
        events_error,
        append_error,
        thread_items_error,
        cancel_error,
        terminal_error,
    ] {
        assert_eq!(error.error.code, INVALID_PARAMS_ERROR_CODE);
        assert_eq!(error.error.message, "request was not found");
    }

    let mut read_mcp = TestAppServer::new_with_env(read_codex_home.path(), &env).await?;
    initialize_as(&mut read_mcp, "caller-1").await?;
    let read_request_id = read_mcp
        .send_raw_request(
            "request/read",
            Some(serde_json::json!({
                "requestId": run_response.request.request_id,
            })),
        )
        .await?;
    let read_response = read_mcp
        .read_stream_until_response_message(RequestId::Integer(read_request_id))
        .await?;
    let read_response = to_response::<RequestReadResponse>(read_response)?;
    let items_request_id = read_mcp
        .send_raw_request(
            "thread/items/list",
            Some(serde_json::json!({
                "threadId": run_response.thread_id,
                "cursor": null,
                "limit": 10,
            })),
        )
        .await?;
    let items_response = read_mcp
        .read_stream_until_response_message(RequestId::Integer(items_request_id))
        .await?;
    let items_response = to_response::<ThreadItemsListResponse>(items_response)?;

    assert_eq!(read_response.request.status, RequestStatus::Running);
    assert_eq!(items_response.data, Vec::<ThreadItemRecord>::new());

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_cancel_and_events_continue_after_app_server_restart_with_mysql() -> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    let mut restarted_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
    restarted_mcp.initialize().await?;
    let cancel_request_id = restarted_mcp
        .send_raw_request(
            "request/cancel",
            Some(serde_json::json!({
                "requestId": run_response.request.request_id,
                "reason": "restart-cancel",
            })),
        )
        .await?;
    let cancel_response = restarted_mcp
        .read_stream_until_response_message(RequestId::Integer(cancel_request_id))
        .await?;
    let cancel_response = to_response::<RequestCancelResponse>(cancel_response)?;
    let events_request_id = restarted_mcp
        .send_raw_request(
            "request/events/list",
            Some(serde_json::json!({
                "requestId": run_response.request.request_id,
                "cursor": null,
                "limit": 10,
            })),
        )
        .await?;
    let events_response = restarted_mcp
        .read_stream_until_response_message(RequestId::Integer(events_request_id))
        .await?;
    let events_response = to_response::<RequestEventsListResponse>(events_response)?;

    assert_eq!(cancel_response.request.status, RequestStatus::Cancelled);
    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running", "request/cancelled"]
    );
    assert_eq!(
        events_response.data.last().map(|event| event.sequence),
        Some(cancel_response.request.latest_event_cursor)
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_events_cursor_pagination_survives_app_server_restart_with_mysql() -> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    {
        let mut restarted_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        restarted_mcp.initialize().await?;
        let cancel_request_id = restarted_mcp
            .send_raw_request(
                "request/cancel",
                Some(serde_json::json!({
                    "requestId": run_response.request.request_id.clone(),
                    "reason": "restart-cancel",
                })),
            )
            .await?;
        let cancel_response = restarted_mcp
            .read_stream_until_response_message(RequestId::Integer(cancel_request_id))
            .await?;
        let cancel_response = to_response::<RequestCancelResponse>(cancel_response)?;
        assert_eq!(cancel_response.request.status, RequestStatus::Cancelled);
    }

    let mut events_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
    events_mcp.initialize().await?;
    let first_page = list_events_page(
        &mut events_mcp,
        &run_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 1,
    )
    .await?;
    let second_page = list_events_page(
        &mut events_mcp,
        &run_response.request.request_id,
        first_page.next_cursor,
        /*limit*/ 1,
    )
    .await?;
    let third_page = list_events_page(
        &mut events_mcp,
        &run_response.request.request_id,
        second_page.next_cursor,
        /*limit*/ 1,
    )
    .await?;

    assert_eq!(
        first_page
            .data
            .iter()
            .map(|event| (event.sequence, event.event_type.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "request/queued")]
    );
    assert_eq!(first_page.next_cursor, Some(1));
    assert_eq!(
        second_page
            .data
            .iter()
            .map(|event| (event.sequence, event.event_type.as_str()))
            .collect::<Vec<_>>(),
        vec![(2, "request/running")]
    );
    assert_eq!(second_page.next_cursor, Some(2));
    assert_eq!(
        third_page
            .data
            .iter()
            .map(|event| (event.sequence, event.event_type.as_str()))
            .collect::<Vec<_>>(),
        vec![(3, "request/cancelled")]
    );
    assert_eq!(third_page.next_cursor, None);

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_run_idempotency_survives_app_server_restart_with_mysql() -> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let input = format!("sha256:{test_id}");
    let idempotency_key = format!("idem-{test_id}");

    let first_run = {
        let mut mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": input,
                    "idempotencyKey": idempotency_key,
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    let mut restarted_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
    restarted_mcp.initialize().await?;
    let replay_request_id = restarted_mcp
        .send_raw_request(
            "request/run",
            Some(serde_json::json!({
                "threadId": thread_id,
                "input": input,
                "idempotencyKey": idempotency_key,
            })),
        )
        .await?;
    let replay_response = restarted_mcp
        .read_stream_until_response_message(RequestId::Integer(replay_request_id))
        .await?;
    let replay_response = to_response::<RequestRunResponse>(replay_response)?;
    let conflict_request_id = restarted_mcp
        .send_raw_request(
            "request/run",
            Some(serde_json::json!({
                "threadId": thread_id,
                "input": format!("{input}:different"),
                "idempotencyKey": idempotency_key,
            })),
        )
        .await?;
    let conflict = restarted_mcp
        .read_stream_until_error_message(RequestId::Integer(conflict_request_id))
        .await?;

    assert_eq!(replay_response, first_run);
    assert_eq!(conflict.error.code, INVALID_PARAMS_ERROR_CODE);
    assert!(
        conflict
            .error
            .message
            .contains("idempotency key was reused with different input"),
        "unexpected error: {conflict:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_run_concurrent_app_servers_initialize_events_once_with_mysql() -> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let first_codex_home = TempDir::new()?;
    let second_codex_home = TempDir::new()?;
    let events_codex_home = TempDir::new()?;
    create_config_toml(first_codex_home.path(), &server.uri())?;
    create_config_toml(second_codex_home.path(), &server.uri())?;
    create_config_toml(events_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let input = format!("sha256:{test_id}");
    let idempotency_key = format!("idem-{test_id}");

    let first_run = async {
        let mut mcp = TestAppServer::new_with_env(first_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": input,
                    "idempotencyKey": idempotency_key,
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)
    };
    let second_run = async {
        let mut mcp = TestAppServer::new_with_env(second_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": input,
                    "idempotencyKey": idempotency_key,
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)
    };
    let (first_response, second_response) = tokio::join!(first_run, second_run);
    let first_response = first_response?;
    let second_response = second_response?;

    let mut events_mcp = TestAppServer::new_with_env(events_codex_home.path(), &env).await?;
    events_mcp.initialize().await?;
    let events_response = list_events_page(
        &mut events_mcp,
        &first_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;

    assert_eq!(
        second_response.request.request_id,
        first_response.request.request_id
    );
    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_run_concurrent_app_servers_conflicting_input_keeps_single_initialized_request_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let first_codex_home = TempDir::new()?;
    let second_codex_home = TempDir::new()?;
    let events_codex_home = TempDir::new()?;
    create_config_toml(first_codex_home.path(), &server.uri())?;
    create_config_toml(second_codex_home.path(), &server.uri())?;
    create_config_toml(events_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let idempotency_key = format!("idem-{test_id}");

    let first_run = async {
        let mut mcp = TestAppServer::new_with_env(first_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": format!("sha256:first-{test_id}"),
                    "idempotencyKey": idempotency_key,
                })),
            )
            .await?;
        read_request_run_result(&mut mcp, request_id).await
    };
    let second_run = async {
        let mut mcp = TestAppServer::new_with_env(second_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": format!("sha256:second-{test_id}"),
                    "idempotencyKey": idempotency_key,
                })),
            )
            .await?;
        read_request_run_result(&mut mcp, request_id).await
    };
    let (first_result, second_result) = tokio::join!(first_run, second_run);
    let results = [first_result?, second_result?];
    let successful = results
        .iter()
        .filter_map(|result| match result {
            RequestRunResult::Response(response) => Some(response),
            RequestRunResult::Error(_) => None,
        })
        .collect::<Vec<_>>();
    let errors = results
        .iter()
        .filter_map(|result| match result {
            RequestRunResult::Response(_) => None,
            RequestRunResult::Error(error) => Some(error),
        })
        .collect::<Vec<_>>();

    let mut events_mcp = TestAppServer::new_with_env(events_codex_home.path(), &env).await?;
    events_mcp.initialize().await?;
    let events_response = list_events_page(
        &mut events_mcp,
        &successful
            .first()
            .expect("one request/run should succeed")
            .request
            .request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;

    assert_eq!(successful.len(), 1);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].error.code, INVALID_PARAMS_ERROR_CODE);
    assert!(
        errors[0]
            .error
            .message
            .contains("idempotency key was reused with different input"),
        "unexpected error: {:?}",
        errors[0]
    );
    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_terminal_completion_survives_cancel_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    {
        let mut terminal_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        terminal_mcp.initialize().await?;
        let terminal_request_id = terminal_mcp
            .send_raw_request(
                "request/terminal",
                Some(serde_json::json!({
                    "requestId": run_response.request.request_id.clone(),
                    "signal": "completed",
                    "payloadInline": null,
                })),
            )
            .await?;
        let terminal_response = terminal_mcp
            .read_stream_until_response_message(RequestId::Integer(terminal_request_id))
            .await?;
        let terminal_response = to_response::<RequestTerminalResponse>(terminal_response)?;
        assert_eq!(terminal_response.request.status, RequestStatus::Completed);
    }

    let mut cancel_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
    cancel_mcp.initialize().await?;
    let cancel_request_id = cancel_mcp
        .send_raw_request(
            "request/cancel",
            Some(serde_json::json!({
                "requestId": run_response.request.request_id.clone(),
                "reason": "late-cancel",
            })),
        )
        .await?;
    let cancel_response = cancel_mcp
        .read_stream_until_response_message(RequestId::Integer(cancel_request_id))
        .await?;
    let cancel_response = to_response::<RequestCancelResponse>(cancel_response)?;
    let events_response = list_events_page(
        &mut cancel_mcp,
        &run_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;

    assert_eq!(cancel_response.request.status, RequestStatus::Completed);
    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running", "request/completed"]
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn runtime_write_denied_releases_writer_lease_for_queued_request_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let run_codex_home = TempDir::new()?;
    let terminal_codex_home = TempDir::new()?;
    let replay_codex_home = TempDir::new()?;
    let read_codex_home = TempDir::new()?;
    create_config_toml(run_codex_home.path(), &server.uri())?;
    create_config_toml(terminal_codex_home.path(), &server.uri())?;
    create_config_toml(replay_codex_home.path(), &server.uri())?;
    create_config_toml(read_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let first_params = serde_json::json!({
        "threadId": thread_id,
        "input": format!("sha256:{test_id}:first"),
        "idempotencyKey": format!("idem-{test_id}-1"),
    });
    let second_params = serde_json::json!({
        "threadId": thread_id,
        "input": format!("sha256:{test_id}:second"),
        "idempotencyKey": format!("idem-{test_id}-2"),
    });

    let (first_run_response, queued_response) = {
        let mut mcp = TestAppServer::new_with_env(run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let first_request_id = mcp
            .send_raw_request("request/run", Some(first_params.clone()))
            .await?;
        let first_response = mcp
            .read_stream_until_response_message(RequestId::Integer(first_request_id))
            .await?;
        let first_response = to_response::<RequestRunResponse>(first_response)?;
        let second_request_id = mcp
            .send_raw_request("request/run", Some(second_params.clone()))
            .await?;
        let second_response = mcp
            .read_stream_until_response_message(RequestId::Integer(second_request_id))
            .await?;
        let second_response = to_response::<RequestRunResponse>(second_response)?;
        (first_response, second_response)
    };

    assert_eq!(first_run_response.request.status, RequestStatus::Running);
    assert!(first_run_response.writer_lease.is_some());
    assert_eq!(queued_response.request.status, RequestStatus::Queued);
    assert!(queued_response.writer_lease.is_none());

    let mut terminal_mcp = TestAppServer::new_with_env(terminal_codex_home.path(), &env).await?;
    terminal_mcp.initialize().await?;
    let terminal_request_id = terminal_mcp
        .send_raw_request(
            "request/terminal",
            Some(serde_json::json!({
                "requestId": first_run_response.request.request_id.clone(),
                "signal": "runtime_write_denied",
                "payloadInline": r#"{"toolName":"apply_patch","reason":"read_only_runtime"}"#,
            })),
        )
        .await?;
    let terminal_response = terminal_mcp
        .read_stream_until_response_message(RequestId::Integer(terminal_request_id))
        .await?;
    let terminal_response = to_response::<RequestTerminalResponse>(terminal_response)?;

    assert_eq!(terminal_response.request.status, RequestStatus::LeaseLost);

    let mut replay_mcp = TestAppServer::new_with_env(replay_codex_home.path(), &env).await?;
    replay_mcp.initialize().await?;
    let replay_request_id = replay_mcp
        .send_raw_request("request/run", Some(second_params))
        .await?;
    let replay_response = replay_mcp
        .read_stream_until_response_message(RequestId::Integer(replay_request_id))
        .await?;
    let replay_response = to_response::<RequestRunResponse>(replay_response)?;

    assert_eq!(
        replay_response.request.request_id,
        queued_response.request.request_id
    );
    assert_eq!(replay_response.request.status, RequestStatus::Running);
    assert!(replay_response.writer_lease.is_some());

    let mut read_mcp = TestAppServer::new_with_env(read_codex_home.path(), &env).await?;
    read_mcp.initialize().await?;
    let events_response = list_events_page(
        &mut read_mcp,
        &first_run_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;

    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running", "request/lease_lost"]
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn owner_lease_timeout_releases_writer_lease_for_queued_request_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let run_codex_home = TempDir::new()?;
    let terminal_codex_home = TempDir::new()?;
    let replay_codex_home = TempDir::new()?;
    let read_codex_home = TempDir::new()?;
    create_config_toml(run_codex_home.path(), &server.uri())?;
    create_config_toml(terminal_codex_home.path(), &server.uri())?;
    create_config_toml(replay_codex_home.path(), &server.uri())?;
    create_config_toml(read_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let first_params = serde_json::json!({
        "threadId": thread_id,
        "input": format!("sha256:{test_id}:first"),
        "idempotencyKey": format!("idem-{test_id}-1"),
    });
    let second_params = serde_json::json!({
        "threadId": thread_id,
        "input": format!("sha256:{test_id}:second"),
        "idempotencyKey": format!("idem-{test_id}-2"),
    });

    let (first_run_response, queued_response) = {
        let mut mcp = TestAppServer::new_with_env(run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let first_request_id = mcp
            .send_raw_request("request/run", Some(first_params.clone()))
            .await?;
        let first_response = mcp
            .read_stream_until_response_message(RequestId::Integer(first_request_id))
            .await?;
        let first_response = to_response::<RequestRunResponse>(first_response)?;
        let second_request_id = mcp
            .send_raw_request("request/run", Some(second_params.clone()))
            .await?;
        let second_response = mcp
            .read_stream_until_response_message(RequestId::Integer(second_request_id))
            .await?;
        let second_response = to_response::<RequestRunResponse>(second_response)?;
        (first_response, second_response)
    };

    assert_eq!(first_run_response.request.status, RequestStatus::Running);
    assert!(first_run_response.writer_lease.is_some());
    assert_eq!(queued_response.request.status, RequestStatus::Queued);
    assert!(queued_response.writer_lease.is_none());

    let mut terminal_mcp = TestAppServer::new_with_env(terminal_codex_home.path(), &env).await?;
    terminal_mcp.initialize().await?;
    let terminal_request_id = terminal_mcp
        .send_raw_request(
            "request/terminal",
            Some(serde_json::json!({
                "requestId": first_run_response.request.request_id.clone(),
                "signal": "owner_lease_timed_out",
                "payloadInline": r#"{"owner":"worker-timeout"}"#,
            })),
        )
        .await?;
    let terminal_response = terminal_mcp
        .read_stream_until_response_message(RequestId::Integer(terminal_request_id))
        .await?;
    let terminal_response = to_response::<RequestTerminalResponse>(terminal_response)?;

    assert_eq!(
        terminal_response.request.status,
        RequestStatus::OwnerTimedOut
    );

    let mut replay_mcp = TestAppServer::new_with_env(replay_codex_home.path(), &env).await?;
    replay_mcp.initialize().await?;
    let replay_request_id = replay_mcp
        .send_raw_request("request/run", Some(second_params))
        .await?;
    let replay_response = replay_mcp
        .read_stream_until_response_message(RequestId::Integer(replay_request_id))
        .await?;
    let replay_response = to_response::<RequestRunResponse>(replay_response)?;

    assert_eq!(
        replay_response.request.request_id,
        queued_response.request.request_id
    );
    assert_eq!(replay_response.request.status, RequestStatus::Running);
    assert!(replay_response.writer_lease.is_some());

    let mut read_mcp = TestAppServer::new_with_env(read_codex_home.path(), &env).await?;
    read_mcp.initialize().await?;
    let events_response = list_events_page(
        &mut read_mcp,
        &first_run_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;

    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "request/queued",
            "request/running",
            "request/owner_timed_out"
        ]
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_terminal_rejects_invalid_payload_without_state_change_with_mysql() -> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    {
        let mut terminal_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
        terminal_mcp.initialize().await?;
        let terminal_request_id = terminal_mcp
            .send_raw_request(
                "request/terminal",
                Some(serde_json::json!({
                    "requestId": run_response.request.request_id.clone(),
                    "signal": "failed",
                    "payloadInline": "not-json",
                })),
            )
            .await?;
        let terminal_error = terminal_mcp
            .read_stream_until_error_message(RequestId::Integer(terminal_request_id))
            .await?;
        assert_eq!(terminal_error.error.code, INVALID_PARAMS_ERROR_CODE);
        assert!(
            terminal_error
                .error
                .message
                .contains("request event payload must be valid JSON"),
            "unexpected error: {terminal_error:?}"
        );
    }

    let mut read_mcp = TestAppServer::new_with_env(codex_home.path(), &env).await?;
    read_mcp.initialize().await?;
    let read_request_id = read_mcp
        .send_raw_request(
            "request/read",
            Some(serde_json::json!({
                "requestId": run_response.request.request_id.clone(),
            })),
        )
        .await?;
    let read_response = read_mcp
        .read_stream_until_response_message(RequestId::Integer(read_request_id))
        .await?;
    let read_response = to_response::<RequestReadResponse>(read_response)?;
    let events_response = list_events_page(
        &mut read_mcp,
        &run_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;

    assert_eq!(read_response.request.status, RequestStatus::Running);
    assert_eq!(
        events_response
            .data
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["request/queued", "request/running"]
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn request_terminal_concurrent_app_servers_append_one_terminal_event_with_mysql() -> Result<()>
{
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let run_codex_home = TempDir::new()?;
    let first_codex_home = TempDir::new()?;
    let second_codex_home = TempDir::new()?;
    let read_codex_home = TempDir::new()?;
    create_config_toml(run_codex_home.path(), &server.uri())?;
    create_config_toml(first_codex_home.path(), &server.uri())?;
    create_config_toml(second_codex_home.path(), &server.uri())?;
    create_config_toml(read_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": format!("thread-{test_id}"),
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };

    let first_terminal = async {
        let mut mcp = TestAppServer::new_with_env(first_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/terminal",
                Some(serde_json::json!({
                    "requestId": run_response.request.request_id.clone(),
                    "signal": "completed",
                    "payloadInline": r#"{"worker":"first"}"#,
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestTerminalResponse>(response)
    };
    let second_terminal = async {
        let mut mcp = TestAppServer::new_with_env(second_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/terminal",
                Some(serde_json::json!({
                    "requestId": run_response.request.request_id.clone(),
                    "signal": "failed",
                    "payloadInline": r#"{"worker":"second"}"#,
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestTerminalResponse>(response)
    };
    let (first_response, second_response) = tokio::join!(first_terminal, second_terminal);
    let first_response = first_response?;
    let second_response = second_response?;

    let mut read_mcp = TestAppServer::new_with_env(read_codex_home.path(), &env).await?;
    read_mcp.initialize().await?;
    let events_response = list_events_page(
        &mut read_mcp,
        &run_response.request.request_id,
        /*cursor*/ None,
        /*limit*/ 10,
    )
    .await?;
    let event_types = events_response
        .data
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        first_response.request.status,
        second_response.request.status
    );
    assert!(matches!(
        first_response.request.status,
        RequestStatus::Completed | RequestStatus::Failed
    ));
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| matches!(**event_type, "request/completed" | "request/failed"))
            .count(),
        1
    );
    assert_eq!(event_types.len(), 3);
    assert_eq!(event_types[0], "request/queued");
    assert_eq!(event_types[1], "request/running");

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn thread_append_with_lease_uses_request_run_lease_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let run_codex_home = TempDir::new()?;
    let append_codex_home = TempDir::new()?;
    let replay_codex_home = TempDir::new()?;
    create_config_toml(run_codex_home.path(), &server.uri())?;
    create_config_toml(append_codex_home.path(), &server.uri())?;
    create_config_toml(replay_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let append_idempotency_key = format!("append-{test_id}");

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };
    let writer_lease = run_response
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    let mut append_mcp = TestAppServer::new_with_env(append_codex_home.path(), &env).await?;
    append_mcp.initialize().await?;
    let append_request_id = append_mcp
        .send_raw_request(
            "thread/appendWithLease",
            Some(serde_json::json!({
                "threadId": run_response.thread_id.clone(),
                "requestId": run_response.request.request_id.clone(),
                "leaseId": writer_lease.lease_id.clone(),
                "writerOwnerToken": writer_lease.writer_owner_token.clone(),
                "fencingToken": writer_lease.fencing_token,
                "appendIdempotencyKey": append_idempotency_key.clone(),
                "expectedThreadVersion": 0,
                "items": [
                    {
                        "payloadRef": format!("object://items/{test_id}"),
                    }
                ],
            })),
        )
        .await?;
    let append_response = append_mcp
        .read_stream_until_response_message(RequestId::Integer(append_request_id))
        .await?;
    let append_response = to_response::<AppendThreadItemsWithLeaseResponse>(append_response)?;

    assert_eq!(append_response.thread_version, 1);
    assert_eq!(append_response.first_append_sequence, 1);
    assert_eq!(append_response.last_append_sequence, 1);
    assert!(!append_response.deduplicated);
    let mut replay_mcp = TestAppServer::new_with_env(replay_codex_home.path(), &env).await?;
    replay_mcp.initialize().await?;
    let replay_request_id = replay_mcp
        .send_raw_request(
            "thread/appendWithLease",
            Some(serde_json::json!({
                "threadId": run_response.thread_id.clone(),
                "requestId": run_response.request.request_id.clone(),
                "leaseId": writer_lease.lease_id,
                "writerOwnerToken": writer_lease.writer_owner_token,
                "fencingToken": writer_lease.fencing_token,
                "appendIdempotencyKey": append_idempotency_key,
                "expectedThreadVersion": 0,
                "items": [
                    {
                        "payloadRef": format!("object://items/{test_id}"),
                    }
                ],
            })),
        )
        .await?;
    let replay_response = replay_mcp
        .read_stream_until_response_message(RequestId::Integer(replay_request_id))
        .await?;
    let replay_response = to_response::<AppendThreadItemsWithLeaseResponse>(replay_response)?;

    assert_eq!(replay_response.thread_version, 1);
    assert_eq!(replay_response.first_append_sequence, 1);
    assert_eq!(replay_response.last_append_sequence, 1);
    assert!(replay_response.deduplicated);
    let items_request_id = append_mcp
        .send_raw_request(
            "thread/items/list",
            Some(serde_json::json!({
                "threadId": run_response.thread_id.clone(),
                "cursor": null,
                "limit": 10,
            })),
        )
        .await?;
    let items_response = append_mcp
        .read_stream_until_response_message(RequestId::Integer(items_request_id))
        .await?;
    let items_response = to_response::<ThreadItemsListResponse>(items_response)?;

    assert_eq!(
        items_response.data,
        vec![ThreadItemRecord {
            thread_id: run_response.thread_id.clone(),
            sequence: 1,
            payload_ref: format!("object://items/{test_id}"),
        }]
    );
    assert_eq!(items_response.next_cursor, None);
    assert_eq!(
        MysqlCloudStateStore::from_database_url(&database_url)
            .await?
            .list_thread_items(
                &run_response.thread_id,
                /*cursor*/ None,
                /*limit*/ 10
            )
            .await?,
        (
            vec![ThreadItemRecord {
                thread_id: run_response.thread_id,
                sequence: 1,
                payload_ref: format!("object://items/{test_id}"),
            }],
            None
        )
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn thread_append_with_stale_expected_version_fails_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let run_codex_home = TempDir::new()?;
    let first_append_codex_home = TempDir::new()?;
    let stale_append_codex_home = TempDir::new()?;
    create_config_toml(run_codex_home.path(), &server.uri())?;
    create_config_toml(first_append_codex_home.path(), &server.uri())?;
    create_config_toml(stale_append_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");

    let run_response = {
        let mut mcp = TestAppServer::new_with_env(run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": format!("sha256:{test_id}"),
                    "idempotencyKey": format!("idem-{test_id}"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };
    let writer_lease = run_response
        .writer_lease
        .clone()
        .expect("running request should return writer lease");

    let mut first_append_mcp =
        TestAppServer::new_with_env(first_append_codex_home.path(), &env).await?;
    first_append_mcp.initialize().await?;
    let first_append_request_id = first_append_mcp
        .send_raw_request(
            "thread/appendWithLease",
            Some(serde_json::json!({
                "threadId": run_response.thread_id,
                "requestId": run_response.request.request_id,
                "leaseId": writer_lease.lease_id,
                "writerOwnerToken": writer_lease.writer_owner_token,
                "fencingToken": writer_lease.fencing_token,
                "appendIdempotencyKey": format!("append-{test_id}-1"),
                "expectedThreadVersion": 0,
                "items": [
                    {
                        "payloadRef": format!("object://items/{test_id}/1"),
                    }
                ],
            })),
        )
        .await?;
    let first_append_response = first_append_mcp
        .read_stream_until_response_message(RequestId::Integer(first_append_request_id))
        .await?;
    let first_append_response =
        to_response::<AppendThreadItemsWithLeaseResponse>(first_append_response)?;
    assert_eq!(first_append_response.thread_version, 1);

    let mut stale_append_mcp =
        TestAppServer::new_with_env(stale_append_codex_home.path(), &env).await?;
    stale_append_mcp.initialize().await?;
    let stale_append_request_id = stale_append_mcp
        .send_raw_request(
            "thread/appendWithLease",
            Some(serde_json::json!({
                "threadId": run_response.thread_id,
                "requestId": run_response.request.request_id,
                "leaseId": writer_lease.lease_id,
                "writerOwnerToken": writer_lease.writer_owner_token,
                "fencingToken": writer_lease.fencing_token,
                "appendIdempotencyKey": format!("append-{test_id}-2"),
                "expectedThreadVersion": 0,
                "items": [
                    {
                        "payloadRef": format!("object://items/{test_id}/2"),
                    }
                ],
            })),
        )
        .await?;
    let stale_append_error = read_error_response(&mut stale_append_mcp, stale_append_request_id)
        .await?
        .error;

    assert_eq!(stale_append_error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        stale_append_error.message,
        "expected thread version does not match current thread version"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn thread_append_idempotency_key_is_scoped_to_original_request_after_app_server_restart_with_mysql()
-> Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let first_run_codex_home = TempDir::new()?;
    let first_append_codex_home = TempDir::new()?;
    let terminal_codex_home = TempDir::new()?;
    let second_run_codex_home = TempDir::new()?;
    let conflict_append_codex_home = TempDir::new()?;
    create_config_toml(first_run_codex_home.path(), &server.uri())?;
    create_config_toml(first_append_codex_home.path(), &server.uri())?;
    create_config_toml(terminal_codex_home.path(), &server.uri())?;
    create_config_toml(second_run_codex_home.path(), &server.uri())?;
    create_config_toml(conflict_append_codex_home.path(), &server.uri())?;
    let env = [(MYSQL_URL_ENV_VAR, Some(database_url.as_str()))];
    let test_id = unique_test_id();
    let thread_id = format!("thread-{test_id}");
    let append_idempotency_key = format!("append-{test_id}");

    let first_run_response = {
        let mut mcp = TestAppServer::new_with_env(first_run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "input": format!("sha256:{test_id}:first"),
                    "idempotencyKey": format!("idem-{test_id}-1"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };
    let first_writer_lease = first_run_response
        .writer_lease
        .clone()
        .expect("first running request should return writer lease");

    let mut first_append_mcp =
        TestAppServer::new_with_env(first_append_codex_home.path(), &env).await?;
    first_append_mcp.initialize().await?;
    let first_append_request_id = first_append_mcp
        .send_raw_request(
            "thread/appendWithLease",
            Some(serde_json::json!({
                "threadId": first_run_response.thread_id.clone(),
                "requestId": first_run_response.request.request_id.clone(),
                "leaseId": first_writer_lease.lease_id.clone(),
                "writerOwnerToken": first_writer_lease.writer_owner_token.clone(),
                "fencingToken": first_writer_lease.fencing_token,
                "appendIdempotencyKey": append_idempotency_key.clone(),
                "expectedThreadVersion": 0,
                "items": [
                    {
                        "payloadRef": format!("object://items/{test_id}/1"),
                    }
                ],
            })),
        )
        .await?;
    let first_append_response = first_append_mcp
        .read_stream_until_response_message(RequestId::Integer(first_append_request_id))
        .await?;
    let first_append_response =
        to_response::<AppendThreadItemsWithLeaseResponse>(first_append_response)?;
    assert_eq!(first_append_response.thread_version, 1);

    let mut terminal_mcp = TestAppServer::new_with_env(terminal_codex_home.path(), &env).await?;
    terminal_mcp.initialize().await?;
    let terminal_request_id = terminal_mcp
        .send_raw_request(
            "request/terminal",
            Some(serde_json::json!({
                "requestId": first_run_response.request.request_id.clone(),
                "signal": "completed",
                "payloadInline": "{}",
            })),
        )
        .await?;
    let terminal_response = terminal_mcp
        .read_stream_until_response_message(RequestId::Integer(terminal_request_id))
        .await?;
    let terminal_response = to_response::<RequestTerminalResponse>(terminal_response)?;
    assert_eq!(terminal_response.request.status, RequestStatus::Completed);

    let second_run_response = {
        let mut mcp = TestAppServer::new_with_env(second_run_codex_home.path(), &env).await?;
        mcp.initialize().await?;
        let request_id = mcp
            .send_raw_request(
                "request/run",
                Some(serde_json::json!({
                    "threadId": first_run_response.thread_id.clone(),
                    "input": format!("sha256:{test_id}:second"),
                    "idempotencyKey": format!("idem-{test_id}-2"),
                })),
            )
            .await?;
        let response = mcp
            .read_stream_until_response_message(RequestId::Integer(request_id))
            .await?;
        to_response::<RequestRunResponse>(response)?
    };
    let second_writer_lease = second_run_response
        .writer_lease
        .clone()
        .expect("second running request should return writer lease");

    let mut conflict_append_mcp =
        TestAppServer::new_with_env(conflict_append_codex_home.path(), &env).await?;
    conflict_append_mcp.initialize().await?;
    let conflict_append_request_id = conflict_append_mcp
        .send_raw_request(
            "thread/appendWithLease",
            Some(serde_json::json!({
                "threadId": second_run_response.thread_id.clone(),
                "requestId": second_run_response.request.request_id.clone(),
                "leaseId": second_writer_lease.lease_id.clone(),
                "writerOwnerToken": second_writer_lease.writer_owner_token.clone(),
                "fencingToken": second_writer_lease.fencing_token,
                "appendIdempotencyKey": append_idempotency_key,
                "expectedThreadVersion": 1,
                "items": [
                    {
                        "payloadRef": format!("object://items/{test_id}/1"),
                    }
                ],
            })),
        )
        .await?;
    let conflict_append_error =
        read_error_response(&mut conflict_append_mcp, conflict_append_request_id)
            .await?
            .error;

    assert_eq!(conflict_append_error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        conflict_append_error.message,
        "idempotency key was reused with different input"
    );

    Ok(())
}

async fn list_events_page(
    mcp: &mut TestAppServer,
    request_id: &str,
    cursor: Option<u64>,
    limit: u32,
) -> Result<RequestEventsListResponse> {
    let request_id = mcp
        .send_raw_request(
            "request/events/list",
            Some(serde_json::json!({
                "requestId": request_id,
                "cursor": cursor,
                "limit": limit,
            })),
        )
        .await?;
    let response = mcp
        .read_stream_until_response_message(RequestId::Integer(request_id))
        .await?;

    to_response::<RequestEventsListResponse>(response)
}

enum RequestRunResult {
    Response(RequestRunResponse),
    Error(JSONRPCError),
}

async fn read_request_run_result(
    mcp: &mut TestAppServer,
    request_id: i64,
) -> Result<RequestRunResult> {
    loop {
        let message = mcp.read_next_message().await?;
        match message {
            JSONRPCMessage::Response(response) if response.id == RequestId::Integer(request_id) => {
                return Ok(RequestRunResult::Response(
                    to_response::<RequestRunResponse>(response)?,
                ));
            }
            JSONRPCMessage::Error(error) if error.id == RequestId::Integer(request_id) => {
                return Ok(RequestRunResult::Error(error));
            }
            _ => {}
        }
    }
}

async fn read_error_response(mcp: &mut TestAppServer, request_id: i64) -> Result<JSONRPCError> {
    loop {
        let message = mcp.read_next_message().await?;
        if let JSONRPCMessage::Error(error) = message
            && error.id == RequestId::Integer(request_id)
        {
            return Ok(error);
        }
    }
}

async fn request_error(
    mcp: &mut TestAppServer,
    method: &str,
    params: serde_json::Value,
) -> Result<JSONRPCError> {
    let request_id = mcp.send_raw_request(method, Some(params)).await?;
    read_error_response(mcp, request_id).await
}

async fn initialize_as(mcp: &mut TestAppServer, client_name: &str) -> Result<()> {
    let initialized = mcp
        .initialize_with_client_info(ClientInfo {
            name: client_name.to_string(),
            title: None,
            version: "0.1.0".to_string(),
        })
        .await?;
    let JSONRPCMessage::Response(_) = initialized else {
        anyhow::bail!("expected initialize response, got {initialized:?}");
    };
    Ok(())
}

fn create_config_toml(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[cloud_runtime]
enabled = true
runtime_profile = "read_only"
state_store = "mysql"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}

fn unique_test_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("mysql-app-server-{nanos}")
}
