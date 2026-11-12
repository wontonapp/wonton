//! 取一个网址：每一跳都重新解析、检查地址，自己连、自己发 HTTP/1.1；跳转最多 3 次。响应体只读到上限为止。

use http_body_util::{BodyExt, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use url::Url;
use wonton_protocol::LinkPreviewFailure;

use super::connect::Net;
use super::{Failure, target};

/// ADR-0065：跳转最多 3 次。
const MAX_REDIRECTS: usize = 3;

/// 有些站点只给认得出的预览抓取器回 Open Graph，所以写明是预览。
pub(super) const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; WontonLinkPreview/",
    env!("CARGO_PKG_VERSION"),
    ")"
);

/// 一次取到的结果：最终网址、响应头和读到的响应体。
pub(super) struct Fetched {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// 读响应体时怎么对待超长：网页截断，图片算失败。
#[derive(Clone, Copy)]
pub(super) enum Overflow {
    Truncate,
    Fail,
}

/// 跟着跳转取 `url`，2xx 才算取到。`accept` 是请求的 `Accept`，`check` 在读响应体之前看响应头和 `limit`（比如类型不对就别读了）。
pub(super) async fn get(
    net: &Net,
    mut url: Url,
    accept: &'static str,
    limit: usize,
    overflow: Overflow,
    check: fn(&HeaderMap, usize) -> Result<(), Failure>,
) -> Result<Fetched, Failure> {
    for hop in 0..=MAX_REDIRECTS {
        let (response, guard) = once(net, &url, accept).await?;
        let status = response.status();
        if is_redirect(status) {
            if hop == MAX_REDIRECTS {
                return Err(upstream(format!("{} 次跳转", MAX_REDIRECTS + 1)));
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| upstream(status.to_string()))?;
            let next = url
                .join(location)
                .map_err(|e| upstream(format!("{location}：{e}")))?;
            url = target::check(next)?;
            drop(guard);
            continue;
        }
        if !status.is_success() {
            return Err(upstream(status.to_string()));
        }
        check(response.headers(), limit)?;
        let headers = response.headers().clone();
        let body = read(response.into_body(), limit, overflow).await?;
        drop(guard);
        return Ok(Fetched { url, headers, body });
    }
    unreachable!("循环在最后一跳之前一定返回")
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

pub(super) fn upstream(detail: String) -> Failure {
    Failure::new(LinkPreviewFailure::UpstreamError, detail)
}

/// 连接任务的看守：请求结束或被超时丢下时把连接一起断掉。
pub(super) struct Connection(tokio::task::JoinHandle<()>);

impl Connection {
    /// 在后台跑连接任务，`Connection` 丢掉时一起断掉。
    pub(super) fn spawn<B>(
        connection: hyper::client::conn::http1::Connection<TokioIo<super::connect::BoxIo>, B>,
    ) -> Self
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error + Send + Sync,
    {
        Self(tokio::spawn(async move {
            let _ = connection.await;
        }))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// 发一跳：先解析、检查地址，再连到检查过的地址。
async fn once(
    net: &Net,
    url: &Url,
    accept: &'static str,
) -> Result<(Response<Incoming>, Connection), Failure> {
    let ips = target::addresses(net, url).await?;
    let host = url.host_str().unwrap_or_default();
    // `host_str` 给 IPv6 带着方括号，SNI 和连接要的是不带的。
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.port_or_known_default().unwrap_or(0);
    let stream = net
        .open(bare, &ips, port, url.scheme() == "https")
        .await
        .map_err(upstream)?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| upstream(crate::http::error_chain(&e)))?;
    let guard = Connection::spawn(connection);
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let request = Request::get(path)
        .header(header::HOST, authority)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, HeaderValue::from_static(accept))
        .body(Empty::<Bytes>::new())
        .map_err(|e| upstream(e.to_string()))?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| upstream(crate::http::error_chain(&e)))?;
    Ok((response, guard))
}

/// 读响应体，最多 `limit` 字节。
async fn read(mut body: Incoming, limit: usize, overflow: Overflow) -> Result<Bytes, Failure> {
    let mut data = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| upstream(crate::http::error_chain(&e)))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        let room = limit - data.len();
        if chunk.len() <= room {
            data.extend_from_slice(&chunk);
            continue;
        }
        match overflow {
            Overflow::Truncate => {
                data.extend_from_slice(&chunk[..room]);
                return Ok(data.into());
            }
            Overflow::Fail => return Err(upstream(format!("超过 {limit} 字节"))),
        }
    }
    Ok(data.into())
}
