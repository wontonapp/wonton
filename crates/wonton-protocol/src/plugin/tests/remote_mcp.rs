//! 远端 MCP 的线格式契约（ADR-0066 第 3、3a 节）：确认页的远端信息、详情里的授权状态、安装请求里的授权码，
//! 以及两类新的失败。

use serde_json::json;

use super::*;

fn preview_with_remote() -> InstallPreview {
    let mut preview = preview();
    preview.remote = Some(RemoteMcpPreview {
        host: "mcp.example.com".into(),
        auth: RemoteAuth::Oauth,
    });
    preview
}

/// 契约：预览里的远端信息原样往返；旧核心没有 `remote` 时读成空。
#[test]
fn remote_preview_round_trips() {
    let preview = preview_with_remote();
    let wire = serde_json::to_value(&preview).unwrap();
    assert_eq!(
        wire["remote"],
        json!({"host": "mcp.example.com", "auth": "oauth"})
    );
    assert_eq!(round_trip(&preview), preview);

    let mut without = serde_json::to_value(preview).unwrap();
    without.as_object_mut().unwrap().remove("remote");
    let decoded: InstallPreview = serde_json::from_value(without).unwrap();
    assert_eq!(decoded.remote, None);
}

/// 契约：详情里的远端信息和授权状态原样往返。
#[test]
fn remote_detail_round_trips() {
    let mut shown = detail();
    shown.remote = Some(RemoteMcpDetail {
        host: "mcp.example.com".into(),
        auth: RemoteAuth::Header,
        authorization: RemoteAuthorization::Authorized,
    });
    assert_eq!(round_trip(&shown), shown);
    assert_eq!(
        serde_json::to_value(&shown).unwrap()["remote"],
        json!({
            "host": "mcp.example.com", "auth": "header", "authorization": "authorized",
        })
    );
}

/// 契约：不认识的授权状态落到 `Other`，不挡整份详情。
#[test]
fn unknown_remote_authorization_is_other() {
    let mut wire = serde_json::to_value(detail()).unwrap();
    wire["remote"] = json!({
        "host": "mcp.example.com", "auth": "oauth", "authorization": "future_state",
    });
    let decoded: PluginDetail = serde_json::from_value(wire).unwrap();
    assert_eq!(
        decoded.remote.unwrap().authorization,
        RemoteAuthorization::Other
    );
}

/// 契约：不认识的鉴权方式和授权状态都落到 `Other`，App 不认识的取值不挡整份详情。
#[test]
fn unknown_remote_auth_is_other() {
    let auth: RemoteAuth = serde_json::from_value(json!("saml")).unwrap();
    assert_eq!(auth, RemoteAuth::Other);
    let mut wire = serde_json::to_value(preview_with_remote()).unwrap();
    wire["remote"]["auth"] = json!("saml");
    let decoded: InstallPreview = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded.remote.unwrap().auth, RemoteAuth::Other);

    let authorization: RemoteAuthorization = serde_json::from_value(json!("expired")).unwrap();
    assert_eq!(authorization, RemoteAuthorization::Other);
}

/// 契约：两类远端失败是 snake_case，解析得回来；不认识的取值仍按现有规则解码失败。
#[test]
fn remote_failures_round_trip() {
    for (failure, wire) in [
        (PluginFailure::RemoteUnavailable, "remote_unavailable"),
        (PluginFailure::RemoteError, "remote_error"),
    ] {
        assert_eq!(serde_json::to_value(failure).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<PluginFailure>(json!(wire)).unwrap(),
            failure
        );
    }
    assert!(
        serde_json::from_value::<PluginFailure>(json!("future_failure")).is_err(),
        "失败类别没有 `other` 兜底"
    );
}

/// 契约：不带 `remote_oauth` 的旧安装请求照样解析；带了就原样往返。
#[test]
fn install_request_remote_oauth_optional() {
    let mut request = InstallRequest {
        source: "github:example/wonton-remote@v1.0.0".into(),
        digest: "sha256:remote".into(),
        plan: String::new(),
        scope: None,
        audience: None,
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: SecretValues::default(),
        settings: SettingValues::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    };
    let old = serde_json::to_value(&request).unwrap();
    let decoded: InstallRequest = serde_json::from_value(old).unwrap();
    assert_eq!(decoded.remote_oauth, None);

    request.remote_oauth = Some(RemoteOauthGrant {
        code: "code-1".into(),
        state: "state-1".into(),
    });
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(
        wire["remote_oauth"],
        json!({"code": "code-1", "state": "state-1"})
    );
    assert_eq!(round_trip(&request), request);
}

/// 契约：授权接口和插件详情的地址由协议给，App 不自己拼；授权起点回的是授权页地址。
#[test]
fn remote_oauth_paths_address_the_owner_only_endpoints() {
    let start = RemoteOauthStart {
        authorize_url: "https://mcp.example.com/authorize?state=s1".into(),
    };
    assert_eq!(
        serde_json::to_value(&start).unwrap(),
        json!({"authorize_url": "https://mcp.example.com/authorize?state=s1"})
    );
    assert_eq!(
        remote_oauth_start_path(),
        "/_wonton/api/plugins/remote-oauth/start"
    );
    assert_eq!(
        plugin_remote_oauth_start_path("io.github.notion"),
        "/_wonton/api/plugins/io.github.notion/remote-oauth/start"
    );
    assert_eq!(
        plugin_remote_oauth_finish_path("io.github.notion"),
        "/_wonton/api/plugins/io.github.notion/remote-oauth/finish"
    );
}
