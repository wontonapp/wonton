//! Matrix 入口按 Bot 认令牌的隧道测试：只有声明 `matrix.bot` 的插件网络能到矩阵接口，令牌认不收时按来源地址。

use std::sync::Arc;

use hyper::{Method, StatusCode};

use super::tunnel_tests::{Home, from_plugin, from_plugin_without_token, header, lan};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::plugin::PluginId;
use wonton_protocol::BotKeeper;

#[tokio::test]
async fn bots_reach_matrix_only_from_networks_of_plugins_with_matrix_bot() {
    let (fake, config) = crate::matrix::fake::start().await;
    fake.state().tokens.insert(
        "bot-token".into(),
        ("@hermes:test".into(), "bot-device".into()),
    );
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let home = Home::start_with(None, Some(Arc::new(matrix))).await;
    let bot = vec!["matrix.bot".parse().unwrap()];
    home.plugins
        .register_source_for_test("app.echo", "172.30.1.0/24", bot);
    home.plugins.bot_records_for_test(
        &PluginId::parse("app.echo").unwrap(),
        &[(
            "default",
            "@hermes:test",
            BotKeeper::User {
                username: "owner".into(),
            },
            "bot-token",
        )],
    );
    home.plugins
        .register_source_for_test("app.web", "172.30.2.0/24", Vec::new());

    let (status, _, body) =
        from_plugin(&home, "172.30.1.5:40000", "/_matrix/client/versions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["versions"].is_array());
    // matrix-sdk 恢复会话前取版本时还没有令牌，这里必须放行；其余客户端接口仍要令牌。
    let (status, _, _) =
        from_plugin_without_token(&home, "172.30.1.5:40000", "/_matrix/client/versions").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = from_plugin_without_token(
        &home,
        "172.30.1.5:40000",
        "/_matrix/client/v3/account/whoami",
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // 规则同 matrix.<base>：只有客户端接口，写法可疑的路径先挡住。
    let admin = "/_synapse/admin/v1/register";
    let (status, _, _) = from_plugin(&home, "172.30.1.5:40000", admin).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let sneaky = "/_matrix/client/../../_synapse/admin/v1/register";
    let (status, _, _) = from_plugin(&home, "172.30.1.5:40000", sneaky).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 没申请 matrix.bot 的插件、插件网络以外的来源都不行。
    let (status, headers, body) =
        from_plugin(&home, "172.30.2.5:40000", "/_matrix/client/versions").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["errcode"], "M_FORBIDDEN");
    assert_eq!(
        header(&headers, "x-wonton-error"),
        Some("permission_denied")
    );
    let (status, headers, _) =
        from_plugin(&home, "192.168.1.20:40000", "/_matrix/client/versions").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(header(&headers, "x-wonton-error"), Some("unauthenticated"));
    let (status, _, _) = lan(
        &home,
        Method::GET,
        "matrix.wonton.internal",
        "/_matrix/client/versions",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "没有来源地址（如经隧道）也不行"
    );
}
