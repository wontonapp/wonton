//! 核心重启后 Bot 的同步要能接着跑：插件正在恢复、问不到 Tuwunel 认令牌时答 503，让上游适配器重试；
//! 真的被吊销的令牌仍答 403。
//!
//! com.nousresearch.hermes 的 Matrix 适配器把 403（以及 `M_UNKNOWN_TOKEN` 这类错误码）当永久错误，
//! `_sync_loop` 直接 return：进程还活着、`/health/detailed` 还报 connected，但再也收不到消息，只有重启
//! 容器才好（`plugin-settings-bot-*` 也不会变）。核心重启时网关先接连接、后核对插件，这中间插件来的请求
//! 不能答 403。

use hyper::StatusCode;

use super::family_rules_tests::Home;
use super::send_limits::SendLimits;

#[tokio::test]
async fn sync_while_plugins_are_restoring_is_retryable() {
    let h = Home::start(SendLimits::default()).await;
    let restoring = h.plugins.restoring_guard();

    let (status, headers, body) = h.bot_sync().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        headers["retry-after"].to_str().unwrap(),
        "5",
        "要告诉上游隔多久再来"
    );

    drop(restoring);
    let (status, _, body) = h.bot_sync().await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn sync_while_tuwunel_cannot_be_asked_is_retryable() {
    let h = Home::start(SendLimits::default()).await;
    h.fake.state().refuse_whoami = true;

    let (status, headers, body) = h.bot_sync().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(headers["retry-after"].to_str().unwrap(), "5");

    h.fake.state().refuse_whoami = false;
    let (status, _, body) = h.bot_sync().await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_revoked_token_still_gets_403() {
    let h = Home::start(SendLimits::default()).await;
    h.fake.state().tokens.clear();

    let (status, _, body) = h.bot_sync().await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], "M_FORBIDDEN");
}

/// 健康检查看的是“还在同步”，不是上游写的 connected：同步停了（插件进程活着、状态文件还新）要看得出来。
#[tokio::test(start_paused = true)]
async fn a_successful_sync_marks_the_plugin_live_until_it_goes_quiet() {
    let h = Home::start(SendLimits::default()).await;
    let id = crate::plugin::PluginId::parse("app.echo").unwrap();
    assert_eq!(h.plugins.stalled_bot_sync(&id), None, "没同步过不算停");

    assert_eq!(h.bot_sync().await.0, StatusCode::OK);
    assert_eq!(h.plugins.stalled_bot_sync(&id), None);

    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    assert_eq!(
        h.plugins.stalled_bot_sync(&id),
        Some(121),
        "两分钟没同步就是停了"
    );

    assert_eq!(h.bot_sync().await.0, StatusCode::OK);
    assert_eq!(h.plugins.stalled_bot_sync(&id), None);
}
