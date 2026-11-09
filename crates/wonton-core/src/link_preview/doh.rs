//! 经主人的代理用 DoH 解析域名（RFC 8484，ADR-0065）：本地 DNS 在国内可能回一个连不上的公网地址，经代理 `CONNECT` 它也没用。
//! DoH 服务器在配了代理时才用，地址可配，默认 `https://1.1.1.1/dns-query`；拿到地址后照旧检查是不是内网地址。

use std::net::IpAddr;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::{self, HeaderValue};
use hyper_util::rt::TokioIo;
use url::{Host, Url};
use wonton_protocol::LinkPreviewFailure;

use super::Failure;
use super::connect::Net;
use super::fetch::{Connection, USER_AGENT};

/// 一条 DNS 响应报文的上限：TCP 上的 DNS 报文最长 64 KiB。
const MAX_RESPONSE: usize = 1 << 16;
/// DoH 单独限时。整次预览 10 秒超时，解析迟迟不回要报解析失败，不能把时间占满。
const TIMEOUT: Duration = Duration::from_secs(5);
const CONTENT_TYPE: &str = "application/dns-message";

/// 问出 `name` 的 A 和 AAAA 地址。两个里有一个拿到地址就算解析成功；都失败时报 [`LinkPreviewFailure::DnsFailed`]。
pub(super) async fn lookup(net: &Net, server: Url, name: &str) -> Result<Vec<IpAddr>, Failure> {
    let (host, port) = endpoint(&server)?;
    let ips = server_addresses(net, &server, &host, port).await?;
    match tokio::time::timeout(TIMEOUT, query(net, &server, &host, port, &ips, name)).await {
        Err(_) => Err(dns(format!("{host} 超过 {TIMEOUT:?}"))),
        Ok(found) => found,
    }
}

/// DoH 服务器的 TLS 名（IPv6 去掉方括号）和端口。
fn endpoint(server: &Url) -> Result<(String, u16), Failure> {
    let host = server
        .host_str()
        .ok_or_else(|| dns(server.to_string()))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = server
        .port_or_known_default()
        .ok_or_else(|| dns(server.to_string()))?;
    Ok((host, port))
}

/// DoH 服务器自己的地址。用 IP 写时不用解析；写成域名时走本地解析（代理到不了本地的解析器，DoH 又还没连上）。
async fn server_addresses(
    net: &Net,
    server: &Url,
    host: &str,
    port: u16,
) -> Result<Vec<IpAddr>, Failure> {
    let found = match server.host() {
        Some(Host::Ipv4(ip)) => vec![IpAddr::V4(ip)],
        Some(Host::Ipv6(ip)) => vec![IpAddr::V6(ip)],
        Some(Host::Domain(name)) => (net.resolve)(name.to_owned(), port)
            .await
            .map_err(|e| dns(format!("{host}：{e}")))?,
        None => return Err(dns(server.to_string())),
    };
    if found.is_empty() {
        return Err(dns(format!("{host} 没有地址")));
    }
    Ok(found)
}

/// A 和 AAAA 都问。一个是 SERVFAIL、另一个有地址时照样用有地址的那个。
async fn query(
    net: &Net,
    server: &Url,
    host: &str,
    port: u16,
    ips: &[IpAddr],
    name: &str,
) -> Result<Vec<IpAddr>, Failure> {
    let mut found = Vec::new();
    let mut failures = Vec::new();
    for record in [RecordType::A, RecordType::AAAA] {
        match exchange_any(net, server, host, port, ips, name, record).await {
            Ok(ips) => found.extend(ips),
            Err(failure) => failures.push(format!("{record}: {}", failure.detail)),
        }
    }
    if found.is_empty() {
        let detail = if failures.is_empty() {
            format!("{name} 没有 A 或 AAAA 地址")
        } else {
            failures.join("；")
        };
        return Err(dns(detail));
    }
    Ok(found)
}

/// 依次试 DoH 服务器的每个地址，第一个回出合法响应的为准。
async fn exchange_any(
    net: &Net,
    server: &Url,
    host: &str,
    port: u16,
    ips: &[IpAddr],
    name: &str,
    record: RecordType,
) -> Result<Vec<IpAddr>, Failure> {
    let mut last = String::new();
    for ip in ips {
        match exchange(net, server, host, *ip, port, name, record).await {
            Ok(answer) => return Ok(answer),
            Err(failure) => last = failure.detail,
        }
    }
    Err(dns(last))
}

/// 向一个 DoH 服务器地址发一条查询（POST，RFC 8484 的 `application/dns-message`），解出报文里的地址。
async fn exchange(
    net: &Net,
    server: &Url,
    host: &str,
    ip: IpAddr,
    port: u16,
    name: &str,
    record: RecordType,
) -> Result<Vec<IpAddr>, Failure> {
    let (id, payload) = question(name, record)?;
    let stream = net
        .open(host, &[ip], port, server.scheme() == "https")
        .await
        .map_err(dns)?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| dns(crate::http::error_chain(&e)))?;
    let guard = Connection::spawn(connection);
    let request = Request::post(path(server))
        .header(header::HOST, authority(server))
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE))
        .header(header::ACCEPT, HeaderValue::from_static(CONTENT_TYPE))
        .body(Full::new(payload))
        .map_err(|e| dns(e.to_string()))?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| dns(crate::http::error_chain(&e)))?;
    if !response.status().is_success() {
        return Err(dns(response.status().to_string()));
    }
    if !is_dns_message(response.headers()) {
        let kind = content_type(response.headers());
        return Err(dns(format!("Content-Type {kind}")));
    }
    let body = read(response.into_body()).await?;
    drop(guard);
    answer(&body, id)
}

/// 请求的路径加查询。
fn path(server: &Url) -> &str {
    &server[url::Position::BeforePath..url::Position::AfterQuery]
}

/// `Host` 头：IPv6 带方括号，默认端口不写。
fn authority(server: &Url) -> String {
    let host = server.host_str().unwrap_or_default();
    match server.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

fn is_dns_message(headers: &hyper::HeaderMap) -> bool {
    content_type(headers).eq_ignore_ascii_case(CONTENT_TYPE)
}

fn content_type(headers: &hyper::HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .unwrap_or_default()
}

/// 编一条查询报文，返回它的 ID 和字节。
fn question(name: &str, record: RecordType) -> Result<(u16, Bytes), Failure> {
    let name = Name::from_utf8(name).map_err(|e| dns(e.to_string()))?;
    let id = u16::try_from(getrandom::u32().expect("系统随机数不可用") >> 16).unwrap_or(0);
    let mut message = Message::new(id, MessageType::Query, OpCode::Query);
    message.metadata.recursion_desired = true;
    message.queries.push(Query::query(name, record));
    let bytes = message.to_vec().map_err(|e| dns(e.to_string()))?;
    Ok((id, Bytes::from(bytes)))
}

/// 解一条响应报文。ID 对不上、不是响应、回了错误码或报文被截断都算失败。
fn answer(body: &[u8], id: u16) -> Result<Vec<IpAddr>, Failure> {
    let message = Message::from_vec(body).map_err(|e| dns(e.to_string()))?;
    if message.metadata.id != id {
        return Err(dns("响应 ID 对不上".into()));
    }
    if message.metadata.message_type != MessageType::Response {
        return Err(dns("不是响应".into()));
    }
    if message.metadata.truncation {
        return Err(dns("响应被截断".into()));
    }
    if message.metadata.response_code != ResponseCode::NoError {
        return Err(dns(format!("{:?}", message.metadata.response_code)));
    }
    Ok(message
        .answers
        .iter()
        .filter_map(|record| match &record.data {
            RData::A(ip) => Some(IpAddr::V4(ip.0)),
            RData::AAAA(ip) => Some(IpAddr::V6(ip.0)),
            _ => None,
        })
        .collect())
}

/// 读响应体，超过上限就失败。
async fn read(mut body: hyper::body::Incoming) -> Result<Bytes, Failure> {
    let mut data = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| dns(crate::http::error_chain(&e)))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        if data.len() + chunk.len() > MAX_RESPONSE {
            return Err(dns(format!("响应超过 {MAX_RESPONSE} 字节")));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data.into())
}

fn dns(detail: String) -> Failure {
    Failure::new(LinkPreviewFailure::DnsFailed, detail)
}
