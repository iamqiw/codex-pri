#![cfg(not(target_os = "windows"))]
#![allow(clippy::expect_used)]

use anyhow::Context;
use core_test_support::responses;
use core_test_support::test_codex_exec::test_codex_exec;
use predicates::str::contains;
use pretty_assertions::assert_eq;
use sqlx::MySqlPool;
use std::fs;
use uuid::Uuid;

const MYSQL_URL_ENV_VAR: &str = "CODEX_CLOUD_STATE_MYSQL_URL";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires CODEX_CLOUD_STATE_MYSQL_URL pointing at an isolated MySQL test database"]
async fn cloud_runtime_exec_uses_request_run_and_persists_to_mysql() -> anyhow::Result<()> {
    let database_url = std::env::var(MYSQL_URL_ENV_VAR)
        .context("CODEX_CLOUD_STATE_MYSQL_URL must be set to run this ignored test")?;
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let body = responses::sse(vec![
        responses::ev_response_created("resp1"),
        responses::ev_assistant_message("m1", "cloud exec completed"),
        responses::ev_completed("resp1"),
    ]);
    let _response_mock = responses::mount_sse_once(&server, body).await;
    let prompt = format!("cloud-runtime-exec-{}", Uuid::new_v4());
    let base = format!("{}/v1", server.uri());
    fs::write(
        test.home_path().join("config.toml"),
        format!(
            r#"openai_base_url = {}

[cloud_runtime]
enabled = true
runtime_profile = "read_only"
state_store = "mysql"
"#,
            serde_json::to_string(&base)?
        ),
    )?;

    test.cmd()
        .env(MYSQL_URL_ENV_VAR, &database_url)
        .arg("--skip-git-repo-check")
        .arg("-C")
        .arg(test.cwd_path())
        .arg("-m")
        .arg("gpt-5.1")
        .arg(&prompt)
        .assert()
        .success()
        .stdout(contains("cloud exec completed"));

    let pool = MySqlPool::connect(&database_url).await?;
    let (request_id, status, turn_id, latest_event_cursor): (String, String, Option<String>, u64) =
        sqlx::query_as(
            r#"SELECT request_id, status, turn_id, latest_event_cursor
FROM cloud_requests
WHERE input_hash = ?
ORDER BY created_at_ms DESC
LIMIT 1"#,
        )
        .bind(&prompt)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "completed");
    assert!(
        turn_id.is_some(),
        "request/run should bind the real turn id"
    );
    assert!(
        latest_event_cursor >= 3,
        "request should advance the durable event cursor"
    );

    let event_types: Vec<String> = sqlx::query_scalar(
        r#"SELECT event_type
FROM cloud_request_events
WHERE request_id = ?
ORDER BY sequence"#,
    )
    .bind(&request_id)
    .fetch_all(&pool)
    .await?;
    assert!(event_types.starts_with(&["request/queued".into(), "request/running".into()]));
    assert!(
        event_types
            .iter()
            .any(|event_type| event_type == "notification/item/completed"),
        "durable request events should include projected turn output"
    );
    assert_eq!(
        event_types.last().map(String::as_str),
        Some("request/completed")
    );

    Ok(())
}
