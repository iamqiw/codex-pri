use super::*;
use crate::config::CloudRuntimeConfig;
use crate::session::tests::make_session_and_context;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;

#[test]
fn parse_csv_supports_quotes_and_commas() {
    let input = "id,name\n1,\"alpha, beta\"\n2,gamma\n";
    let (headers, rows) = parse_csv(input).expect("csv parse");
    assert_eq!(headers, vec!["id".to_string(), "name".to_string()]);
    assert_eq!(
        rows,
        vec![
            vec!["1".to_string(), "alpha, beta".to_string()],
            vec!["2".to_string(), "gamma".to_string()]
        ]
    );
}

#[test]
fn csv_escape_quotes_when_needed() {
    assert_eq!(csv_escape("simple"), "simple");
    assert_eq!(csv_escape("a,b"), "\"a,b\"");
    assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
}

#[test]
fn render_instruction_template_expands_placeholders_and_escapes_braces() {
    let row = json!({
        "path": "src/lib.rs",
        "area": "test",
        "file path": "docs/readme.md",
    });
    let rendered = render_instruction_template(
        "Review {path} in {area}. Also see {file path}. Use {{literal}}.",
        &row,
    );
    assert_eq!(
        rendered,
        "Review src/lib.rs in test. Also see docs/readme.md. Use {literal}."
    );
}

#[test]
fn render_instruction_template_leaves_unknown_placeholders() {
    let row = json!({
        "path": "src/lib.rs",
    });
    let rendered = render_instruction_template("Check {path} then {missing}", &row);
    assert_eq!(rendered, "Check src/lib.rs then {missing}");
}

#[test]
fn ensure_unique_headers_rejects_duplicates() {
    let headers = vec!["path".to_string(), "path".to_string()];
    let Err(err) = ensure_unique_headers(headers.as_slice()) else {
        panic!("expected duplicate header error");
    };
    assert_eq!(
        err,
        FunctionCallError::RespondToModel("csv header path is duplicated".to_string())
    );
}

#[tokio::test]
async fn cloud_runtime_read_only_rejects_spawn_agents_on_csv() {
    let (session, mut turn) = make_session_and_context().await;
    let mut config = (*turn.config).clone();
    config.cloud_runtime = CloudRuntimeConfig {
        enabled: true,
        ..Default::default()
    };
    turn.config = Arc::new(config);

    let Err(err) = spawn_agents_on_csv::handle(
        Arc::new(session),
        Arc::new(turn),
        json!({
            "csv_path": "missing.csv",
            "instruction": "work"
        })
        .to_string(),
    )
    .await
    else {
        panic!("spawn_agents_on_csv should be disabled");
    };

    assert_eq!(
        err,
        FunctionCallError::RespondToModel(
            "spawn_agents_on_csv is disabled for cloud runtime read-only profile".to_string(),
        )
    );
}
