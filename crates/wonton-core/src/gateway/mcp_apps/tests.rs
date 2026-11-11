//! MCP Apps 的宿主页（#35）：CSP 只放开资源声明的域名、资源必须是 HTML。

use rmcp::model::ResourceContents;
use serde_json::json;

use super::{HOST_PAGE, MAX_RESOURCE, frame_policy, host_policy, html};

#[test]
fn the_frame_policy_opens_only_declared_domains() {
    let csp = json!({
        "connectDomains": ["https://api.example.com/path", "not a url"],
        "resourceDomains": ["https://cdn.example.com"],
        "frameDomains": ["https://frame.example.com"],
        "baseUriDomains": ["https://base.example.com"],
    });
    let policy = frame_policy(Some(&csp));
    // 界面是 opaque origin：不给 'self'，只能打声明的域名，核心接口都经宿主页。
    assert!(policy.contains("connect-src https://api.example.com"));
    assert!(!policy.contains("connect-src 'self'"));
    assert!(host_policy().contains("connect-src 'self'"));
    assert!(!HOST_PAGE.contains("allow-same-origin"));
    assert!(policy.contains("script-src 'unsafe-inline' https://cdn.example.com"));
    assert!(policy.contains("frame-src 'self' about: data: https://frame.example.com"));
    assert!(policy.contains("base-uri https://base.example.com"));
    assert!(!policy.contains("not a url"));
    assert!(frame_policy(None).contains("default-src 'none'"));
}

#[test]
fn resources_must_be_html_text_and_fit() {
    let text = |mime: &str, body: String| ResourceContents::TextResourceContents {
        uri: "ui://x".into(),
        mime_type: Some(mime.into()),
        text: body,
        meta: None,
    };
    assert_eq!(
        html(&[text("text/html", "<p>ok</p>".into())]).unwrap(),
        "<p>ok</p>"
    );
    assert!(html(&[text("text/plain", "ok".into())]).is_err());
    assert!(html(&[text("text/html;profile=mcp-app", String::new())]).is_ok());
    assert!(html(&[text("text/html", "x".repeat(MAX_RESOURCE + 1))]).is_err());
    assert!(html(&[ResourceContents::blob("YWJj", "ui://x")]).is_err());
    assert!(html(&[]).is_err());
}
