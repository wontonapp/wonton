//! Tuwunel 的推送入口只绑核心私有网络，和公开网关一起启动、退出。

use std::net::SocketAddr;

use anyhow::Context;
use axum::Router;
use axum::serve::ListenerExt;
use tokio::net::TcpListener;
use wonton_core::gateway::matrix_rtc_alias::{self, AliasPeer, TlsListener};

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
    listener: wonton_core::entry_runtime::MainListener,
    app: Router,
    push: Option<(TcpListener, Router)>,
    entry: Option<(TcpListener, Router)>,
    matrix_rtc: Option<(TlsListener, Router)>,
) -> anyhow::Result<()> {
    let gateway = async move {
        axum::serve(
            listener.tap_io(|_| {}),
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
    let entry = async move {
        let Some((listener, app)) = entry else {
            return Ok(());
        };
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(super::shutdown())
        .await
        .context("联邦入口监听异常退出")
    };
    let matrix_rtc = async move {
        let Some((listener, app)) = matrix_rtc else {
            return Ok(());
        };
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<AliasPeer>(),
        )
        .with_graceful_shutdown(super::shutdown())
        .await
        .context("matrix_rtc 别名监听异常退出")
    };
    // 任一监听失败都让核心退出，不留健康的状态页掩盖入口丢失。
    tokio::try_join!(gateway, push, entry, matrix_rtc)?;
    Ok(())
}

/// `matrix_rtc` 提供方网络里服务器名别名的 TLS 监听（ADR-0096）。没接 Matrix 时不开：没有服务器名可答。
pub(crate) async fn bind_matrix_rtc(
    configured: Option<&str>,
    server_name: Option<&str>,
) -> anyhow::Result<Option<TlsListener>> {
    let Some(listen) = configured else {
        tracing::info!("没有设置 WONTON_MATRIX_RTC_LISTEN，通话插件验不了 OpenID 令牌");
        return Ok(None);
    };
    let Some(server_name) = server_name else {
        tracing::info!("核心没接 Matrix，不开 matrix_rtc 别名监听");
        return Ok(None);
    };
    let tcp = TcpListener::bind(listen)
        .await
        .with_context(|| format!("监听 WONTON_MATRIX_RTC_LISTEN 的 {listen}"))?;
    tracing::info!(listen = %tcp.local_addr()?, "matrix_rtc 别名监听已启动");
    Ok(Some(TlsListener {
        tcp,
        acceptor: matrix_rtc_alias::acceptor(server_name)?,
    }))
}
