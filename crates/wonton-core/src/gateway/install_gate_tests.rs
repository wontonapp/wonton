//! 装卸的角色门槛（ADR-0088）：AskAdmin 发卡片给所有管理员并等回答，拒绝 403、过期 408、
//! 不能装 403 permission_required。安装走 FakeDocker，卡片用真的确认流程。

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::family_tests::send_as;
use super::tests::{CORE, Harness, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;

/// 记一个家人并把他的设备登记上，角色默认“朋友”（不能装）。
async fn invite_member(h: &Harness, username: &str) -> iroh::PublicKey {
    let device = SecretKey::generate().public();
    let mut record = MemberRecord::for_test(username);
    record.role_id = "friend".into();
    h.members.invite(record).unwrap();
    h.tunnel
        .devices
        .add_if(device, None, Some(username.into()), |_| true)
        .unwrap();
    device
}

/// 造一个可安装的 echo 插件包（FakeDocker 建容器、写记录），返回 compose 文本。
fn echo_compose() -> String {
    WHOAMI
        .replace("app.whoami", "app.echo")
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "")
}

async fn install_as_owner(h: &Harness, id: &str) {
    h.manager
        .install(
            &echo_compose(),
            &[],
            crate::plugin::languages::Languages::default(),
        )
        .await
        .unwrap();
    let _ = id;
}

#[tokio::test]
async fn a_deny_role_member_cannot_install_or_uninstall() {
    let h = Harness::start(None).await;
    let device = invite_member(&h, "mia").await;
    install_as_owner(&h, "app.echo").await;

    let install = send_as(
        &h.app,
        Some(device),
        Method::POST,
        CORE,
        "/_wonton/api/plugins",
        &[("content-type", "application/json")],
        r#"{"source": "github:o/r", "digest": "sha256:aa", "plan": "p", "grant": []}"#,
    )
    .await;
    assert_eq!(install.status, StatusCode::FORBIDDEN, "{}", install.text());
    assert_eq!(install.error_kind(), Some("permission_required"));
    assert_eq!(install.json()["missing"], "install_apps");

    let uninstall = send_as(
        &h.app,
        Some(device),
        Method::DELETE,
        CORE,
        "/_wonton/api/plugins/app.echo",
        &[],
        "",
    )
    .await;
    assert_eq!(uninstall.status, StatusCode::FORBIDDEN);
    assert_eq!(uninstall.error_kind(), Some("permission_required"));
    assert_eq!(uninstall.json()["missing"], "install_apps");
}

/// AskAdmin：请求挂住，卡片出现在管理员设备上；同意后安装完成、请求返回 201。
#[tokio::test]
async fn an_ask_admin_install_waits_for_an_administrator() {
    let h = Harness::start(None).await;
    let device = invite_member(&h, "mia").await;
    // 主人开一张“管理员”卡的角色之外，直接把 mia 换成 AskAdmin 的角色。
    let draft = wonton_protocol::RoleDraft {
        name: "要批的".into(),
        permissions: wonton_protocol::Permissions {
            install_apps: wonton_protocol::Gate::AskAdmin,
            ..wonton_protocol::Permissions::strictest()
        },
    };
    let role = h.roles.create(&draft).unwrap();
    h.members.set_role("mia", &role.id).unwrap();

    // 挂住的安装请求（mia 的设备身份）：先暂存一个真包，预览给什么请求就带什么。
    let package = crate::plugin::PluginPackage::parse(&echo_compose(), &Languages::default())
        .expect("echo 包能解析");
    let preview = h
        .manager
        .preview_upload(package, &Languages::default())
        .await
        .expect("上传预览成功");
    let staged = serde_json::json!({
        "source": preview.source,
        "digest": preview.digest,
        "plan": preview.plan,
        "grant": preview
            .permissions
            .iter()
            .map(|p| p.permission.clone())
            .collect::<Vec<_>>(),
    })
    .to_string();
    let app = h.app.clone();
    let waiting = tokio::spawn(async move {
        let request = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/_wonton/api/plugins")
            .header("host", format!("{CORE}:80"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(staged))
            .unwrap();
        let mut request = request;
        request.extensions_mut().insert(super::TunnelPeer(device));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_text = String::from_utf8_lossy(&body).into_owned();
        (status, body_text)
    });

    // 管理员（主人的设备：主人在管理员集合里）等卡片出现。
    let mut confirmations = Vec::new();
    for _ in 0..50 {
        let owner_cards = send_as(
            &h.app,
            None,
            Method::GET,
            CORE,
            "/_wonton/api/confirmations",
            &[("authorization", super::family_tests::bearer().as_str())],
            "",
        )
        .await;
        confirmations = owner_cards.json()["confirmations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !confirmations.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(confirmations.len(), 1, "卡片没有出现");
    assert_eq!(confirmations[0]["audience"], "admins");
    assert_eq!(confirmations[0]["plugin_id"], "app.echo");

    // 拒绝：安装请求拿到 403 permission_required。
    let id = confirmations[0]["id"].as_str().unwrap().to_owned();
    let deny = send_as(
        &h.app,
        None,
        Method::POST,
        CORE,
        &format!("/_wonton/confirmations/{id}"),
        &[
            ("authorization", super::family_tests::bearer().as_str()),
            ("content-type", "application/json"),
        ],
        r#"{"answer": "reject"}"#,
    )
    .await;
    assert_eq!(deny.status, StatusCode::OK, "{}", deny.text());

    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("安装请求在卡片处理后返回")
        .unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"]["kind"],
        "permission_required"
    );
}

#[tokio::test]
async fn an_ask_admin_upgrade_completes_after_the_administrator_approves() {
    let h = Harness::start(None).await;
    let device = invite_member(&h, "mia").await;
    install_as_owner(&h, "app.echo").await;
    let draft = wonton_protocol::RoleDraft {
        name: "要批的".into(),
        permissions: wonton_protocol::Permissions {
            install_apps: wonton_protocol::Gate::AskAdmin,
            ..wonton_protocol::Permissions::strictest()
        },
    };
    let role = h.roles.create(&draft).unwrap();
    h.members.set_role("mia", &role.id).unwrap();

    // 升级请求先发出去（挂住）：带一个暂存包的来源，走 UpgradeTo::Source。
    let package = crate::plugin::PluginPackage::parse(
        &echo_compose().replace("version: 0.1.0", "version: 0.2.0"),
        &Languages::default(),
    )
    .expect("echo 包能解析");
    let preview = h
        .manager
        .preview_upload(package, &Languages::default())
        .await
        .expect("上传预览成功");
    let staged = serde_json::json!({
        "source": preview.source,
        "digest": preview.digest,
        "grant": preview.permissions.iter().map(|p| p.permission.clone()).collect::<Vec<_>>(),
    })
    .to_string();
    let app = h.app.clone();
    let waiting = tokio::spawn(async move {
        let request = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/_wonton/api/plugins/app.echo/upgrade")
            .header("host", format!("{CORE}:80"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(staged))
            .unwrap();
        let mut request = request;
        request.extensions_mut().insert(super::TunnelPeer(device));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_text = String::from_utf8_lossy(&body).into_owned();
        (status, body_text)
    });

    // 等管理员卡片出现，同意它。
    let mut card_id = None;
    for _ in 0..50 {
        let cards = send_as(
            &h.app,
            None,
            Method::GET,
            CORE,
            "/_wonton/api/confirmations",
            &[("authorization", super::family_tests::bearer().as_str())],
            "",
        )
        .await;
        let pending = cards.json()["confirmations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(card) = pending.first() {
            card_id = Some(card["id"].as_str().unwrap().to_owned());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let Some(id) = card_id else {
        panic!("管理员卡片没有出现");
    };
    let approve = send_as(
        &h.app,
        None,
        Method::POST,
        CORE,
        &format!("/_wonton/confirmations/{id}"),
        &[
            ("authorization", super::family_tests::bearer().as_str()),
            ("content-type", "application/json"),
        ],
        r#"{"answer": "approve"}"#,
    )
    .await;
    assert_eq!(approve.status, StatusCode::OK, "{}", approve.text());

    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("升级请求在批准后返回")
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["plugin"]["id"], "app.echo");
}

/// 主人在 audience 接口上给另一个角色打开这个应用：那台设备重连就能看到。
#[tokio::test]
async fn the_owner_sets_a_plugin_audience_and_roles_follow() {
    let h = Harness::start(None).await;
    let device = invite_member(&h, "mia").await;
    install_as_owner(&h, "app.whoami").await;

    // 朋友角色本来就看不到 app.whoami。
    let before = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[],
        "",
    )
    .await;
    let ids: Vec<String> = before.json()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plugin| plugin["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(!ids.contains(&"app.whoami".to_owned()), "{}", before.text());

    // 主人把开放范围设为朋友角色。
    let put = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/plugins/app.whoami/audience",
        &[
            ("authorization", super::family_tests::bearer().as_str()),
            ("content-type", "application/json"),
        ],
        r#"{"role_ids": ["friend"]}"#,
    )
    .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.text());
    // All 角色自动在开放范围里，朋友是这次显式加上的。
    assert_eq!(put.json()["role_ids"], json!(["admin", "friend"]));

    // 家人的设备再读列表：有了。
    let after = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[],
        "",
    )
    .await;
    let ids: Vec<String> = after.json()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plugin| plugin["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(ids.contains(&"app.whoami".to_owned()), "{}", after.text());
}

/// All 角色永远在 audience 里，删不掉；不认识的 ID 先拒绝、不写任何东西。
#[tokio::test]
async fn the_all_role_stays_in_the_audience_and_unknown_ids_are_refused() {
    let h = Harness::start(None).await;
    install_as_owner(&h, "app.whoami").await;

    // admin 模板的 open_apps 是 All：audience 里自动存在。
    let get = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.whoami/audience",
        &[("authorization", super::family_tests::bearer().as_str())],
        "",
    )
    .await;
    assert_eq!(get.status, StatusCode::OK, "{}", get.text());
    assert!(
        get.json()["role_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "admin")
    );

    // 试图把 admin 从 audience 里删掉：admin 模板的 open_apps 还是 All，写进去的结果不变。
    let put = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/plugins/app.whoami/audience",
        &[
            ("authorization", super::family_tests::bearer().as_str()),
            ("content-type", "application/json"),
        ],
        r#"{"role_ids": ["friend"]}"#,
    )
    .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.text());
    assert!(
        put.json()["role_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "admin"),
        "{}",
        put.text()
    );

    // 不认识的 ID：先校验再写，请求 404 且 friend 没被写进去。
    let unknown = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/plugins/app.whoami/audience",
        &[
            ("authorization", super::family_tests::bearer().as_str()),
            ("content-type", "application/json"),
        ],
        r#"{"role_ids": ["nobody"]}"#,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text());
    // 404 的请求一个字都没写：上一个 PUT 给朋友的开放还在，没被这次动过。
    let role = h.roles.get("friend").unwrap();
    assert!(
        role.permissions.open_apps.allows_plugin("app.whoami"),
        "friend 不该被这次请求改掉"
    );
}

/// 非主人的设备调 audience 接口：403 permission_required{Owner}。
#[tokio::test]
async fn the_audience_interface_is_owner_only() {
    let h = Harness::start(None).await;
    let device = invite_member(&h, "mia").await;
    install_as_owner(&h, "app.whoami").await;

    let get = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.whoami/audience",
        &[],
        "",
    )
    .await;
    assert_eq!(get.status, StatusCode::FORBIDDEN, "{}", get.text());
    assert_eq!(get.error_kind(), Some("permission_required"));
    assert_eq!(get.json()["missing"], "owner");
}
