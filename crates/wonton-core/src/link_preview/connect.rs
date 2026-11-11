//! 连到对方站点：用核心自己解析出的 IP 去连（直连，或者经主人的代理 `CONNECT <ip>:<port>`），TLS 的 SNI 仍是原域名。
//! 这样解析和连接之间换不了地址（DNS rebinding）。

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};

use futures_util::future::BoxFuture;
use hyper::Uri;
use hyper::header::HeaderValue;
use hyper_util::client::legacy::connect::proxy::{SocksV5, Tunnel};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tower::ServiceExt;
use url::Url;

use crate::http::error_chain;
use crate::plugin::egress::core_proxy::CoreProxy;

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub(crate) type BoxIo = Box<dyn Io>;

/// 解析域名：主机名和端口进，地址出。
pub(crate) type Resolve =
    Arc<dyn Fn(String, u16) -> BoxFuture<'static, io::Result<Vec<IpAddr>>> + Send + Sync>;
/// 直连时连到对方的 TCP。测试换成把假的公网地址接到本机的站点。
pub(crate) type Dial =
    Arc<dyn Fn(SocketAddr) -> BoxFuture<'static, io::Result<TcpStream>> + Send + Sync>;

/// 出网要用的东西。
#[derive(Clone)]
pub(crate) struct Net {
    pub resolve: Resolve,
    pub dial: Dial,
    /// 主人配的出网代理（ADR-0023），每次连接时取当前的值。
    pub proxy: CoreProxy,
    /// 不给就在第一次用时按系统的根证书建。
    pub tls: Option<Arc<ClientConfig>>,
}

impl Net {
    pub(crate) fn system(proxy: CoreProxy) -> Self {
        Self {
            resolve: Arc::new(|host, port| {
                Box::pin(async move {
                    let found = tokio::net::lookup_host((host.as_str(), port)).await?;
                    Ok(found.map(|addr| addr.ip()).collect())
                })
            }),
            dial: Arc::new(|addr| Box::pin(TcpStream::connect(addr))),
            proxy,
            tls: None,
        }
    }

    /// 解析域名。配了主人代理时经代理用 DoH：国内本地 DNS 可能回得不到的假地址，经代理连它也没用（ADR-0065）。
    /// 没配代理时用本地解析。
    pub(super) async fn resolve(
        &self,
        host: String,
        port: u16,
    ) -> Result<Vec<IpAddr>, super::Failure> {
        match self.proxy.current() {
            None => (self.resolve)(host, port).await.map_err(|e| {
                super::Failure::new(super::LinkPreviewFailure::UpstreamError, e.to_string())
            }),
            Some(_) => super::doh::lookup(self, self.proxy.doh(), &host).await,
        }
    }

    /// 连到 `host` 的某个地址，`tls` 时在上面握 TLS（SNI 和证书校验都按 `host`）。依次试 `ips`，第一个连上的为准。
    pub(super) async fn open(
        &self,
        host: &str,
        ips: &[IpAddr],
        port: u16,
        tls: bool,
    ) -> Result<BoxIo, String> {
        let proxy = self.proxy.current();
        let mut last = String::new();
        for ip in ips {
            let addr = SocketAddr::new(*ip, port);
            let stream = match &proxy {
                None => (self.dial)(addr)
                    .await
                    .map(|stream| Box::new(stream) as BoxIo)
                    .map_err(|e| e.to_string()),
                Some(proxy) => self.through_proxy(proxy, addr).await,
            };
            match stream {
                Ok(stream) if tls => return self.tls(host, stream).await,
                Ok(stream) => return Ok(stream),
                Err(e) => last = format!("{addr}：{e}"),
            }
        }
        Err(last)
    }

    async fn tls(&self, host: &str, stream: BoxIo) -> Result<BoxIo, String> {
        let name = ServerName::try_from(host.to_owned()).map_err(|e| e.to_string())?;
        let stream = TlsConnector::from(self.tls_config()?)
            .connect(name, stream)
            .await
            .map_err(|e| error_chain(&e))?;
        Ok(Box::new(stream))
    }

    fn tls_config(&self) -> Result<Arc<ClientConfig>, String> {
        if let Some(config) = &self.tls {
            return Ok(config.clone());
        }
        static SYSTEM: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
        SYSTEM.get_or_init(system_tls).clone()
    }
}

/// 根证书用系统的，和推送用的 reqwest 一样（rustls-platform-verifier）；加密实现明确用 aws-lc-rs，依赖里也有 ring，
/// 不指定的话 rustls 选不出默认的。
fn system_tls() -> Result<Arc<ClientConfig>, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_platform_verifier()
        .map_err(|e| e.to_string())?
        .with_no_client_auth();
    Ok(Arc::new(config))
}

impl Net {
    /// 经主人的代理连到 `addr`：http(s) 代理发 `CONNECT <ip>:<port>`，socks5 代理发 IP 地址，都不把域名交给代理解析。
    async fn through_proxy(&self, proxy: &Url, addr: SocketAddr) -> Result<BoxIo, String> {
        let host = proxy.host_str().unwrap_or_default().to_owned();
        let port = proxy.port_or_known_default().unwrap_or(1080);
        let tls = match proxy.scheme() {
            "https" => Some(self.tls_config()?),
            _ => None,
        };
        let to_proxy = tower::service_fn(move |_: Uri| {
            let (host, tls) = (host.clone(), tls.clone());
            proxy_stream(host, port, tls)
        });
        // `Tunnel`、`SocksV5` 只拿这个地址传给 `to_proxy`，真正连哪里由 `to_proxy` 定。
        let proxy_uri = Uri::from_static("http://proxy");
        let target: Uri = format!("http://{addr}")
            .parse()
            .map_err(|e: hyper::http::uri::InvalidUri| e.to_string())?;
        let user = decoded(proxy.username());
        let password = proxy.password().map(decoded);
        let authenticated = !user.is_empty() || password.is_some();
        let stream = if proxy.scheme() == "socks5" {
            let mut socks = SocksV5::new(proxy_uri, to_proxy);
            if authenticated {
                socks = socks.with_auth(user, password.unwrap_or_default());
            }
            // SocksError 的 Display 不带原因，Debug 带。
            socks.oneshot(target).await.map_err(|e| format!("{e:?}"))?
        } else {
            let mut tunnel = Tunnel::new(proxy_uri, to_proxy);
            if authenticated {
                tunnel = tunnel.with_auth(basic(&user, password.as_deref())?);
            }
            tunnel.oneshot(target).await.map_err(|e| error_chain(&e))?
        };
        Ok(stream.into_inner())
    }
}

/// 拿走 `host` 而不是借用：借用的话 `service_fn` 里的 future 推不出对所有生命周期都是 `Send`，axum 的处理函数要求它是。
async fn proxy_stream(
    host: String,
    port: u16,
    tls: Option<Arc<ClientConfig>>,
) -> io::Result<TokioIo<BoxIo>> {
    let stream = TcpStream::connect(format!("{host}:{port}")).await?;
    let Some(tls) = tls else {
        return Ok(TokioIo::new(Box::new(stream)));
    };
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let name = ServerName::try_from(bare).map_err(io::Error::other)?;
    let stream = TlsConnector::from(tls).connect(name, stream).await?;
    Ok(TokioIo::new(Box::new(stream)))
}

fn decoded(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

fn basic(user: &str, password: Option<&str>) -> Result<HeaderValue, String> {
    use base64::Engine;
    let pair = format!("{user}:{}", password.unwrap_or_default());
    let encoded = base64::engine::general_purpose::STANDARD.encode(pair);
    HeaderValue::from_str(&format!("Basic {encoded}")).map_err(|e| e.to_string())
}
