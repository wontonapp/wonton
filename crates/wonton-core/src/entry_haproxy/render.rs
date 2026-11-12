//! 从 [`Model`] 生成 `haproxy.cfg` 和两份 crt-list。纯字符串拼接，不碰 Docker，也不解释网络。
//!
//! 两条前端各认一个固定对端：公网前端只认连接器，局域网前端只认局域网转发。规则顺序有讲究：
//! `reject` 在 `expect-proxy` 前面——解析 PROXY 头以后 `src` 变成客户端地址，再按 `src` 判连接器
//! 就会把连接器自己拒掉。

use super::model::{
    AUTH_FAILURE_HEADER, AUTH_FAILURES_EXPIRE, AUTH_FAILURES_LIMIT, AUTH_FAILURES_SIZE, CERT_FILE,
    GATEWAY_ADDRESS, LAN_BIND, LAN_CRT_LIST, Model, PAIRED_BIND, PUBLIC_BIND, PUBLIC_CRT_LIST,
    RUNTIME_SOCKET, Trust,
};

pub struct Rendered {
    pub config: String,
    pub public_crt_list: String,
    pub lan_crt_list: String,
}

pub fn render(model: &Model) -> Result<Rendered, String> {
    model.validate()?;
    let mut config = String::new();
    preamble(&mut config);
    tables(&mut config);
    public_frontend(&mut config, model);
    if let Some(forward) = &model.lan_forward {
        lan_frontend(&mut config, model, forward);
    }
    if model.paired_federation {
        paired_frontend(&mut config, &model.suffix);
    }
    gateway(&mut config);
    Ok(Rendered {
        config,
        public_crt_list: crt_list(&model.public_names()),
        lan_crt_list: crt_list(&model.lan_names()),
    })
}

/// crt-list 的一行：一张证书（证书和私钥在同一个 PEM 里）后面跟它伺候的名字；
/// `strict-sni` 下列表外的 SNI 回 `unrecognized_name`。没有名字时是空文件，握手全拒。
fn crt_list(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    format!("{CERT_FILE} {}\n", names.join(" "))
}

fn preamble(out: &mut String) {
    out.push_str("global\n  log stdout format raw local0\n  maxconn 1024\n");
    out.push_str(&format!(
        "  stats socket {RUNTIME_SOCKET} level operator\n\n"
    ));
    out.push_str(
        "defaults\n  mode http\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\n  option http-restrict-req-hdr-names delete\n\nresolvers docker\n  nameserver docker 127.0.0.11:53\n  resolve_retries 1\n  timeout resolve 1s\n  timeout retry 1s\n  hold valid 10s\n\n",
    );
}

/// 失败计数的两张表：IPv4 按单个地址，IPv6 按 /64（同一个 /64 共用一个桶）。
fn tables(out: &mut String) {
    let limit = format!("size {AUTH_FAILURES_SIZE} expire {AUTH_FAILURES_EXPIRE} store gpc0");
    out.push_str(&format!(
        "backend auth_failures4\n  stick-table type ip {limit}\n\nbackend auth_failures6\n  stick-table type ipv6 {limit}\n\n"
    ));
}

fn public_frontend(out: &mut String, model: &Model) {
    // 公网前端怎么解析真实来源由连接器决定：PROXY v2 才等 PROXY 头，头模式不解析。
    frontend(
        out,
        "public",
        PUBLIC_BIND,
        PUBLIC_CRT_LIST,
        model,
        &model.connectors.join(" "),
        matches!(model.trust, Trust::ProxyV2),
    );
}

/// 局域网转发永远带 PROXY v2（ADR-0021“网络规则”），和公网连接器用哪种来源写法无关。
fn lan_frontend(out: &mut String, model: &Model, forward: &str) {
    frontend(out, "lan", LAN_BIND, LAN_CRT_LIST, model, forward, true);
}

#[allow(clippy::too_many_arguments)] // 一条前端要的名字、监听、证书列表、对端和来源写法都不同。
fn frontend(
    out: &mut String,
    name: &str,
    bind: &str,
    crt_list: &str,
    model: &Model,
    peer: &str,
    proxy_header: bool,
) {
    out.push_str(&format!(
        "frontend {name}\n  bind {bind} ssl crt-list {crt_list} strict-sni\n"
    ));
    // 先按 TCP 层的真实来源认对端：这时的 src 还是对端地址，PROXY 头还没解析。
    out.push_str(&format!(
        "  tcp-request connection reject unless {{ src {peer} }}\n"
    ));
    if proxy_header {
        out.push_str(&format!(
            "  tcp-request connection expect-proxy layer4 if {{ src {peer} }}\n"
        ));
    }
    // 头模式只在公网前端出现；局域网前端从 PROXY 头拿来源。
    let header = match (name, &model.trust) {
        ("public", Trust::Header(header)) => Some(header.as_str()),
        _ => None,
    };
    request_rules(out, name, header, peer);
    response_rules(out);
    out.push_str("  default_backend gateway\n\n");
}

fn request_rules(out: &mut String, entry: &str, header: Option<&str>, peer: &str) {
    if let Some(header) = header {
        out.push_str(&format!(
            "  http-request set-src hdr({header}) if {{ src {peer} }}\n"
        ));
    }
    out.push_str("  acl src_v4 src 0.0.0.0/0\n  acl src_v6 src ::/0\n");
    out.push_str("  http-request track-sc0 src table auth_failures4 if src_v4\n");
    out.push_str("  http-request track-sc1 src,ipmask(32,64) table auth_failures6 if src_v6\n");
    out.push_str(&format!(
        "  acl banned_v4 sc_get_gpc0(0) gt {AUTH_FAILURES_LIMIT}\n  acl banned_v6 sc_get_gpc0(1) gt {AUTH_FAILURES_LIMIT}\n  http-request deny deny_status 429 if banned_v4 or banned_v6\n"
    ));
    // Host 必须和 SNI 相同，带端口的形式（`:443`、发布端口）先去掉端口再比。
    out.push_str("  http-request set-var(txn.sni) ssl_fc_sni,lower\n");
    out.push_str("  http-request deny deny_status 421 unless { hdr(host),field(1,:),lower,strcmp(txn.sni) eq 0 }\n");
    stripped_headers(out);
    out.push_str(&format!(
        "  http-request set-header X-Real-IP %[src]\n  http-request set-header X-Wonton-Entry {entry}\n"
    ));
}

/// 客户端带的和真实来源、入口有关的头全删：不删的话客户端能伪造来源、伪造入口类型，
/// 也能自己加一个计数头。
fn stripped_headers(out: &mut String) {
    for header in [
        "Forwarded",
        "X-Forwarded-For",
        "X-Forwarded-Host",
        "X-Forwarded-Proto",
        "X-Forwarded-Port",
        "X-Forwarded-Prefix",
        "X-Real-IP",
        "X-Wonton-Entry",
        AUTH_FAILURE_HEADER,
    ] {
        out.push_str(&format!("  http-request del-header {header}\n"));
    }
}

fn response_rules(out: &mut String) {
    out.push_str("  http-response track-sc0 src table auth_failures4 if src_v4\n");
    out.push_str("  http-response track-sc1 src,ipmask(32,64) table auth_failures6 if src_v6\n");
    for tracker in 0..2 {
        out.push_str(&format!(
            "  http-response sc-inc-gpc0({tracker}) if {{ res.hdr({AUTH_FAILURE_HEADER}) -m found }}\n"
        ));
    }
    out.push_str(&format!(
        "  http-response del-header {AUTH_FAILURE_HEADER}\n"
    ));
}

/// 已配对的云吞之间：联邦服务器名固定，把 `Host` 归一成它。
fn paired_frontend(out: &mut String, suffix: &str) {
    out.push_str(&format!(
        "frontend paired\n  bind {PAIRED_BIND} ssl crt {CERT_FILE} strict-sni\n  acl correct_sni ssl_fc_sni -i {suffix}\n  http-request deny deny_status 421 unless correct_sni\n  http-request deny deny_status 421 unless {{ hdr(host),lower -i {suffix} {suffix}:443 {suffix}:8448 }}\n  http-request set-header Host {suffix}\n  default_backend gateway\n\n"
    ));
}

fn gateway(out: &mut String) {
    out.push_str(&format!(
        "backend gateway\n  server core {GATEWAY_ADDRESS} resolvers docker init-addr libc,none\n"
    ));
}
