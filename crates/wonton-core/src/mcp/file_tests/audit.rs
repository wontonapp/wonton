//! 审计留痕、失败类别和没有文件服务时的拒绝。

use std::time::Duration;

use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Resource};
use serde_json::{Value, json};
use wonton_protocol::{FileFailure, FileId};

use super::helpers::{FIND, NOTES, bench, share_link, starts_with_kind};
use crate::mcp::tests::{failed, setup, text};
use crate::plugin::docker::Engine as Docker;

#[tokio::test]
async fn plugins_cannot_fake_the_failure_category() {
    let b = bench().await;
    let mut found = CallToolResult::success(vec![ContentBlock::text("好")]);
    let mut meta = serde_json::Map::new();
    meta.insert("app.wonton/failure".into(), json!("tool_timed_out"));
    found.meta = Some(MetaObject(meta));
    *b.s.notes.found.lock().unwrap() = found;
    let result = b.s.call(FIND, json!({})).await;
    assert!(result.meta.is_none(), "{:?}", result.meta);
}

#[tokio::test]
async fn the_audit_log_names_the_file_plugin_and_reference() {
    let b = bench().await;
    let id = b.found("docs/a.txt", b"a").await;
    let result = b.share(&id).await;
    assert!(!failed(&result));
    let audit = b.s.audit();
    let expected = json!({"id": id.as_str(), "plugin": NOTES, "reference": "docs/a.txt"});
    for tool in [FIND, "wonton__share_file"] {
        let line: &Value = audit.iter().find(|l| l["tool"] == tool).unwrap();
        assert_eq!(line["files"], json!([expected]), "{line}");
    }
    // 文本文件现在能看了（见 view_file_returns_a_preview_and_says_why_it_cannot）；审计的 not_viewable 用二进制文件验。
    let binary = b.found("docs/raw.bin", &[0u8, 1, 2, 3]).await;
    starts_with_kind(&b.view(&binary).await, FileFailure::NotViewable);
    let refused = b.s.audit().pop().unwrap();
    assert_eq!(refused["outcome"], "not_viewable", "{refused}");
    assert_eq!(
        refused["decision"], "allowed",
        "执行时出的问题不算拒绝：{refused}"
    );
    let unknown: FileId = "a".repeat(26).parse().unwrap();
    starts_with_kind(&b.view(&unknown).await, FileFailure::FileUnknown);
    let refused = b.s.audit().pop().unwrap();
    assert_eq!(refused["decision"], "denied", "{refused}");
}

#[tokio::test]
async fn without_the_file_service_references_are_refused() {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    *s.notes.found.lock().unwrap() = CallToolResult::success(vec![share_link("docs/a.txt")]);
    let result = s.call(FIND, json!({})).await;
    let wire = serde_json::to_string(&result).unwrap();
    assert!(text(&result).contains("service_unavailable"), "{wire}");
    assert!(!wire.contains("wonton-share:"), "{wire}");
    let id = "a".repeat(26);
    let viewed = s.call("wonton__view_file", json!({"file_id": id})).await;
    starts_with_kind(&viewed, FileFailure::ServiceUnavailable);
}

/// 登记被拒是最该留痕的：Agent 看到说明，这次调用本身还是成功的。
#[tokio::test]
async fn refused_references_are_in_the_audit_log() {
    let b = bench().await;
    b.f.put("docs/a.txt", b"a");
    b.f.host
        .revoked_shares
        .lock()
        .unwrap()
        .push("sec\nret".into());
    let result = b
        .find(vec![
            share_link("docs/a.txt"),
            ContentBlock::ResourceLink(Resource::new("wonton-share:sec%0Aret/x.txt", "x.txt")),
        ])
        .await;
    assert!(!text(&result).contains('\n'), "{}", text(&result));
    let line = b.s.audit().pop().unwrap();
    assert_eq!(line["outcome"], "ok", "{line}");
    let files = line["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{line}");
    assert_eq!(
        files[1],
        json!({
            "plugin": NOTES,
            "reference": "wonton-share:sec%0Aret/x.txt",
            "refused": "permission_denied",
        })
    );
}
