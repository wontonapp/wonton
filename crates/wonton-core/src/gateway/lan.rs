//! 局域网状态页：浏览器用服务器的 IP 地址打开核心的端口时看到的页面，显示运行状态，服务器上还没有设备时显示
//! 首次配对的二维码（ADR-0009 的“接口：扫码配对”）。
//!
//! 只回应 `Host` 是 IP 地址的请求：别的网站把自己的域名解析到局域网 IP（DNS 重绑定）后，浏览器发来的 `Host`
//! 是那个域名，读不到这里的配对码。

use std::net::{IpAddr, SocketAddr};

use axum::extract::Request;
use axum::response::{Html, IntoResponse, Response};
use hyper::header::{CACHE_CONTROL, HeaderValue};
use hyper::{Method, StatusCode};
use qrcode::QrCode;
use qrcode::render::svg;
use url::Url;
use wonton_tunnel::pair::{PairingCode, PairingSecret};

use super::{Gateway, pages};
use crate::pairing::IssuedCode;
use crate::plugin::PluginStatus;

/// 状态页多久自己刷新一次：码快过期、或者刚被扫走时，页面跟着换。
const REFRESH_SECS: u32 = 20;

pub fn request(gateway: &Gateway, host_ip: IpAddr, req: &Request) -> Response {
    if req.method() != Method::GET || req.uri().path() != "/" {
        return pages::error(StatusCode::NOT_FOUND, "not_found", "没有这个页面");
    }
    let code = gateway
        .tunnel
        .pairing
        .first_code()
        .map(|issued| pairing_code(gateway, host_ip, issued));
    let mut response = Html(page(gateway, code.as_ref())).into_response();
    // 码是一次性的，页面不能被缓存。
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// 浏览器能用 `host_ip` 打开这个页面，同一个局域网里的手机也就能用它连隧道的端口。
fn pairing_code(gateway: &Gateway, host_ip: IpAddr, issued: IssuedCode) -> (PairingCode, u64) {
    let direct_addrs = vec![SocketAddr::new(host_ip, gateway.tunnel.port)];
    let code = code_for(gateway, direct_addrs, issued.secret);
    (code, issued.expires_in.as_secs())
}

/// 码里的服务端地址：endpoint ID、全部中继地址（家中继在前，`Tunnel::relay_urls`），加上给定的直连地址。
pub(super) fn code_for(
    gateway: &Gateway,
    direct_addrs: Vec<SocketAddr>,
    secret: PairingSecret,
) -> PairingCode {
    PairingCode {
        server_id: gateway.tunnel.endpoint.id(),
        relay_urls: gateway.tunnel.relay_urls(),
        direct_addrs,
        base_domain: gateway.public.base_domain.clone(),
        core_url: core_url(gateway),
        secret,
    }
}

pub(super) fn core_url(gateway: &Gateway) -> Url {
    Url::parse(&gateway.public.origin(gateway.public.core_host())).expect("核心地址总能解析")
}

fn page(gateway: &Gateway, code: Option<&(PairingCode, u64)>) -> String {
    let pairing = match code {
        Some((code, expires_in)) => {
            let svg = QrCode::new(code.to_uri().as_bytes())
                .expect("配对码放得进二维码")
                .render::<svg::Color<'_>>()
                .min_dimensions(240, 240)
                .build();
            // 渲染出来的是独立的 SVG 文件，嵌进 HTML 时去掉开头的 XML 声明。
            let svg = svg.find("<svg").map_or(svg.as_str(), |start| &svg[start..]);
            format!(
                "<h2>配对</h2><p>用云吞 App 扫这个二维码，手机就连上了这台服务器。</p>\
                 <div class=\"qr\">{svg}</div>\
                 <p class=\"muted\">只能用一次，{} 分钟内有效；页面会自己换新码。第一台设备配上后，这里不再显示二维码。</p>\
                 <details><summary>没法扫码？</summary><p>在 App 里粘贴这段配对码：</p>\
                 <p><code class=\"pairing-code\">{}</code></p></details>",
                expires_in.div_ceil(60),
                pages::escape(&code.to_uri())
            )
        }
        None => format!(
            "<h2>配对</h2><p>已配对 {} 台设备。要再加设备，在已配对的 App 里操作。</p>",
            gateway.tunnel.devices.list().len()
        ),
    };
    let body = format!(
        "<h1>云吞</h1><h2>运行状态</h2><ul>{status}</ul>{pairing}",
        status = status_items(gateway)
    );
    format!(
        "<!doctype html><html lang=\"zh\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"{REFRESH_SECS}\">\
         <title>云吞</title><style>\
         body{{font-family:system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem;color:#222}}\
         li{{margin:.4rem 0}}.muted{{color:#777}}.qr svg{{width:16rem;height:16rem}}\
         </style></head><body>{body}</body></html>"
    )
}

/// 只给数字和就绪与否：局域网里谁都打得开这个页面，不列出主人装了哪些插件。随核心附带的插件每台主机都有，没在运行
/// 时逐个写明在装还是出了什么错（比如拉不下镜像）。
fn status_items(gateway: &Gateway) -> String {
    let ready = |ok: bool| if ok { "就绪" } else { "没有接入" };
    let plugins = gateway.plugins.list();
    let running = plugins
        .iter()
        .filter(|p| p.status == PluginStatus::Running)
        .count();
    let degraded = plugins
        .iter()
        .filter(|p| matches!(p.status, PluginStatus::Degraded { .. }))
        .count();
    let mut items = vec![
        format!(
            "核心：运行中（版本 {}）",
            pages::escape(env!("CARGO_PKG_VERSION"))
        ),
        format!("Matrix：{}", ready(gateway.matrix.is_some())),
        format!("单点登录：{}", ready(gateway.hydra.is_some())),
        format!("应用：{running} 个运行中"),
    ];
    if degraded > 0 {
        items.push(format!("<strong>{degraded} 个应用异常</strong>"));
    }
    if let Some(problem) = gateway.plugins.egress_problem() {
        let message = pages::escape(&problem.message);
        items.push(format!("<strong>出网代理：{message}</strong>"));
    }
    items.extend(gateway.plugins.bundled_not_running().iter().map(|p| {
        // 原因的第一行说清是什么错。没通过健康检查时后面跟着容器日志，Agent 的日志里可能有聊天内容，只在主人登录后
        // 的插件列表里给。
        let state = pages::state_of(&p.status);
        format!(
            "{}：{}",
            pages::escape(&pages::title_of(p)),
            pages::escape(state.lines().next().unwrap_or_default())
        )
    }));
    items
        .iter()
        .map(|item| format!("<li>{item}</li>"))
        .collect()
}
