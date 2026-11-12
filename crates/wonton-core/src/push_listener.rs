//! Tuwunel 的推送入口只绑核心私有网络，和公开网关一起启动、退出。

use std::net::SocketAddr;

use anyhow::Context;
use axum::Router;
use tokio::net::TcpListener;

pub(crate) async fn bind(configured: Option<&str>) -> anyhow::Result<Option<TcpListener>> {
    let Some(listen) = configured else {
        tracing::info!("没有设置 WONTON_PUSH_LISTEN，不开专用推送监听");
        return Ok(None);
    };
    // Compose 的别名只在 core 网络上，有 IPv4 时优先用它，与 Tuwunel 解析到的地址一致。
    let mut addresses = tokio::net::lookup_host(listen)
        .await
        .with_context(|| format!("解析 WONTON_PUSH_LISTEN 的地址 {listen}"))?;
    let first = addresses
        .next()
        .with_context(|| format!("WONTON_PUSH_LISTEN 的 {listen} 解析不出地址"))?;
    let address = if first.is_ipv4() {
        first
    } else {
        addresses.find(SocketAddr::is_ipv4).unwrap_or(first)
    };
    // Docker 29.7.2 容器默认 ip_unprivileged_port_start=0：UID 65534、cap_drop=ALL 实测可绑 80，无需宿主 sysctl。
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("监听推送地址 {listen}（{address}）"))?;
    tracing::info!(listen = %listener.local_addr()?, "推送专用监听已启动");
    Ok(Some(listener))
}

pub(crate) async fn serve(
    listener: TcpListener,
    app: Router,
    push: Option<(TcpListener, Router)>,
) -> anyhow::Result<()> {
    let gateway = async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(super::shutdown())
        .await
        .context("网关异常退出")
    };
    let push = async move {
        let Some((listener, app)) = push else {
            return Ok(());
        };
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(super::shutdown())
        .await
        .context("推送监听异常退出")
    };
    // 任一监听失败都让核心退出，不能留下健康的公开网关却静默丢失推送。
    tokio::try_join!(gateway, push)?;
    Ok(())
}
