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
async fn sync_before_the_source_is_registered_is_retryable() {
    let h = Home::start(SendLimits::default()).await;
    // 来源是每个插件轮到它恢复时才登记的，排在后面的 Bot 的请求先落到网关：这时还认不出来源。
    h.plugins
        .forget_source(&crate::plugin::PluginId::parse("app.echo").unwrap());
    let restoring = h.plugins.restoring_guard();

    let (status, headers, body) = h.bot_sync().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(headers["retry-after"].to_str().unwrap(), "5");

    drop(restoring);
    let (status, _, body) = h.bot_sync().await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "恢复完还认不出才是真的错：{body}"
    );
}

#[tokio::test]
async fn overlapping_restores_stay_restoring_until_the_last_one_ends() {
    let h = Home::start(SendLimits::default()).await;
    let first = h.plugins.restoring_guard();
    let second = h.plugins.restoring_guard();
    drop(first);
    assert_eq!(
        h.bot_sync().await.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "还有一段没恢复完"
    );
    drop(second);
    assert_eq!(h.bot_sync().await.0, StatusCode::OK);
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
    let err = h.plugins.check_bot_sync(&id).unwrap_err();
    assert!(
        err.to_string().contains("121"),
        "报错要说清多久没同步：{err}"
    );

    assert_eq!(h.bot_sync().await.0, StatusCode::OK);
    assert_eq!(h.plugins.stalled_bot_sync(&id), None);
    assert!(h.plugins.check_bot_sync(&id).is_ok());
}

/// 停用一阵再启动、重装，旧实例的同步时间不能算到新实例头上。
#[tokio::test(start_paused = true)]
async fn a_new_instance_starts_counting_from_now() {
    let h = Home::start(SendLimits::default()).await;
    let id = crate::plugin::PluginId::parse("app.echo").unwrap();
    assert_eq!(h.bot_sync().await.0, StatusCode::OK);
    tokio::time::advance(std::time::Duration::from_secs(125)).await;
    assert!(h.plugins.check_bot_sync(&id).is_err(), "先确认旧时间会误报");

    h.plugins.forget_bot_sync(&id);
    assert_eq!(h.plugins.stalled_bot_sync(&id), None);
    assert!(h.plugins.check_bot_sync(&id).is_ok(), "新实例不该被判异常");

    // 一次都还没同步过就不催；同步上来之后照常从那一刻计时。
    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    assert!(h.plugins.check_bot_sync(&id).is_ok());
    assert_eq!(h.bot_sync().await.0, StatusCode::OK);
    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    assert!(h.plugins.check_bot_sync(&id).is_err());
}
