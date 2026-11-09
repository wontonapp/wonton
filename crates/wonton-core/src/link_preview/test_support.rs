//! 链接预览测试的假世界：假 DNS 把域名指到给定的地址，拨号把它们都接到本机的假站点，时钟手动拨。
//! 不连真实网络，也不依赖 Docker。

use std::collections::HashMap;
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::middleware::{self, Next};
use hickory_proto::op::{Message, OpCode, ResponseCode};
use hickory_proto::rr::{RData, Record, RecordType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::{Clock, Limits, LinkPreviews, Net};
use crate::plugin::egress::core_proxy::CoreProxy;

/// 假站点对外的“公网”地址（文档网段，预览不拦它）。
pub(crate) const SITE_IP: &str = "203.0.113.10";

/// 假 DoH 服务器在代理替身里的端口。
pub(crate) const DOH_PORT: u16 = 8443;

/// 假 DoH 对一个域名的回答。
#[derive(Clone)]
pub(crate) enum DohReply {
    Addresses(Vec<IpAddr>),
    Rcode(ResponseCode),
}

/// 测试里看得到的东西。
#[derive(Clone)]
pub(crate) struct Handles {
    /// 假站点收到的请求：`Host` 头加路径。
    pub hits: Arc<Mutex<Vec<String>>>,
    /// 直连时拨过的地址（经代理时不拨）。
    pub dialed: Arc<Mutex<Vec<SocketAddr>>>,
    pub clock: Arc<AtomicU64>,
    pub proxy: CoreProxy,
    /// 假站点在本机的真实地址：代理替身连它。
    pub site: SocketAddr,
    /// 假 DoH 服务器在本机的真实地址，代理替身按端口连它。
    pub doh: SocketAddr,
    /// 本地解析器被问过的名字。
    pub local_names: Arc<Mutex<Vec<String>>>,
    /// 假 DoH 服务器收到的查询数。
    pub doh_hits: Arc<AtomicUsize>,
    /// 假 DoH 的应答表。
    pub doh_zones: Arc<Mutex<HashMap<(String, RecordType), DohReply>>>,
    doh_stop: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// 缓存目录，句柄在它就在。
    pub _dir: Arc<tempfile::TempDir>,
}

impl Handles {
    pub fn advance(&self, by: Duration) {
        self.clock
            .fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }

    /// 这条路径被请求过几次。
    pub fn hits_of(&self, path: &str) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|hit| hit.split_once(' ').is_some_and(|(_, p)| p == path))
            .count()
    }

    pub fn hit_count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }

    /// 假 DoH 对一个域名的回答，家族不匹配时给空的 `NoError`。
    pub fn doh_answer(&self, name: &str, reply: DohReply) {
        let record = match &reply {
            DohReply::Addresses(ips) if ips.iter().any(IpAddr::is_ipv6) => RecordType::AAAA,
            DohReply::Addresses(_) => RecordType::A,
            DohReply::Rcode(_) => RecordType::A,
        };
        let mut zones = self.doh_zones.lock().unwrap();
        zones.insert((normalize(name), record), reply.clone());
        if record == RecordType::A {
            zones
                .entry((normalize(name), RecordType::AAAA))
                .or_insert(DohReply::Addresses(Vec::new()));
        }
    }

    /// 让 DoH 服务器连不上：关掉监听。
    pub fn stop_doh(&self) {
        if let Some(task) = self.doh_stop.lock().unwrap().take() {
            task.abort();
        }
    }

    /// 本地解析器问过 `name` 几次。
    pub fn local_calls(&self, name: &str) -> usize {
        self.local_names
            .lock()
            .unwrap()
            .iter()
            .filter(|asked| *asked == name)
            .count()
    }
}

fn normalize(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// 只认 `site.test` 和 `www.site.test` 的世界。
pub(crate) async fn world(routes: Router) -> (LinkPreviews, Handles) {
    world_with(routes, &[], Limits::default(), None).await
}

/// 假的 https 站点：地址和它的证书。
pub(crate) type TlsSite = (SocketAddr, CertificateDer<'static>);

/// `dns` 补充或覆盖域名的地址；`tls_site` 给了的话，443 和 8443 端口拨到它。世界里总有假 DoH 服务器，代理按端口 8443 连它。
pub(crate) async fn world_with(
    routes: Router,
    dns: &[(&str, &[&str])],
    limits: Limits,
    tls_site: Option<TlsSite>,
) -> (LinkPreviews, Handles) {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let site = serve(routes, hits.clone()).await;
    let mut names: HashMap<String, Vec<IpAddr>> = HashMap::new();
    for name in ["site.test", "www.site.test"] {
        names.insert(name.to_owned(), vec![SITE_IP.parse().unwrap()]);
    }
    for (name, ips) in dns {
        let ips = ips.iter().map(|ip| ip.parse().unwrap()).collect();
        names.insert((*name).to_owned(), ips);
    }
    let names = Arc::new(names);
    let dialed = Arc::new(Mutex::new(Vec::new()));
    let seen = dialed.clone();
    let local_names = Arc::new(Mutex::new(Vec::new()));
    let asked = local_names.clone();
    let proxy = CoreProxy::default();
    let mut roots = rustls::RootCertStore::empty();
    let doh_hits = Arc::new(AtomicUsize::new(0));
    let doh_zones = Arc::new(Mutex::new(HashMap::new()));
    for name in ["site.test", "www.site.test"] {
        doh_zones.lock().unwrap().insert(
            (name.to_owned(), RecordType::A),
            DohReply::Addresses(vec![SITE_IP.parse().unwrap()]),
        );
    }
    let (doh, doh_stop) = doh_server(doh_hits.clone(), doh_zones.clone(), &mut roots).await;
    if let Some((_, cert)) = &tls_site {
        roots.add(cert.clone()).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tls_site = tls_site.map(|(addr, _)| addr);
    proxy.set_doh(
        format!("https://{SITE_IP}:{DOH_PORT}/dns-query")
            .parse()
            .unwrap(),
    );
    let net = Net {
        resolve: Arc::new(move |name, _port| {
            asked.lock().unwrap().push(name.clone());
            let found = names.get(&name).cloned();
            Box::pin(async move {
                found.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, name))
            })
        }),
        dial: Arc::new(move |addr: SocketAddr| {
            seen.lock().unwrap().push(addr);
            let to = match tls_site {
                Some(tls) if matches!(addr.port(), 443 | 8443) => tls,
                _ => site,
            };
            Box::pin(TcpStream::connect(to))
        }),
        proxy: proxy.clone(),
        tls: Some(Arc::new(client)),
    };
    let clock = Arc::new(AtomicU64::new(1_800_000_000_000));
    let now = clock.clone();
    let now: Clock = Arc::new(move || now.load(Ordering::SeqCst));
    let dir = Arc::new(tempfile::tempdir().unwrap());
    let previews = LinkPreviews::with(dir.path().join("link-previews"), net, limits, now);
    let handles = Handles {
        hits,
        dialed,
        clock,
        proxy,
        site,
        doh,
        local_names,
        doh_hits,
        doh_zones,
        doh_stop: Arc::new(Mutex::new(Some(doh_stop))),
        _dir: dir,
    };
    (previews, handles)
}

/// 假 DoH 服务器：TLS 上按 `zones` 回答 RFC 8484 的 POST 查询，证书给 `SITE_IP`（地址用 IP 写）。
async fn doh_server(
    hits: Arc<AtomicUsize>,
    zones: Arc<Mutex<HashMap<(String, RecordType), DohReply>>>,
    roots: &mut rustls::RootCertStore,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let key = rcgen::generate_simple_self_signed(vec![SITE_IP.to_owned()]).unwrap();
    let cert = key.cert.der().clone();
    roots.add(cert.clone()).unwrap();
    let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], private)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let (hits, zones) = (hits.clone(), zones.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let head = read_head(&mut tls).await;
                let length = header_value(&head, "content-length")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0; length];
                if tls.read_exact(&mut body).await.is_err() {
                    return;
                }
                hits.fetch_add(1, Ordering::SeqCst);
                let reply = doh_reply(&body, &zones);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.len()
                );
                let _ = tls.write_all(head.as_bytes()).await;
                let _ = tls.write_all(&reply).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    (addr, task)
}

/// 按 `zones` 编一条 DNS 响应；没登记的名字回 `NXDOMAIN`。
fn doh_reply(body: &[u8], zones: &Mutex<HashMap<(String, RecordType), DohReply>>) -> Vec<u8> {
    let query = Message::from_vec(body).unwrap();
    let question = query.queries.first().unwrap().clone();
    let mut message = Message::response(query.metadata.id, OpCode::Query);
    message.queries.push(question.clone());
    let zone = zones
        .lock()
        .unwrap()
        .get(&(normalize(&question.name.to_ascii()), question.query_type))
        .cloned();
    match zone {
        Some(DohReply::Addresses(ips)) => {
            for ip in ips.iter().filter(|ip| {
                matches!(
                    (ip, question.query_type),
                    (IpAddr::V4(_), RecordType::A) | (IpAddr::V6(_), RecordType::AAAA)
                )
            }) {
                let rdata = match ip {
                    IpAddr::V4(ip) => RData::A((*ip).into()),
                    IpAddr::V6(ip) => RData::AAAA((*ip).into()),
                };
                message
                    .answers
                    .push(Record::from_rdata(question.name.clone(), 60, rdata));
            }
        }
        Some(DohReply::Rcode(code)) => message.metadata.response_code = code,
        None => message.metadata.response_code = ResponseCode::NXDomain,
    }
    message.to_vec().unwrap()
}

/// 读到请求头结束（空行）为止，返回读到的文字。
pub(crate) async fn read_head<S: AsyncReadExt + Unpin>(stream: &mut S) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    String::from_utf8_lossy(&head).into_owned()
}

pub(crate) fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

/// 起假站点，记下每个请求的 `Host` 和路径。
async fn serve(routes: Router, hits: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    let record = move |req: Request, next: Next| {
        let hits = hits.clone();
        async move {
            let host = req
                .headers()
                .get(hyper::header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            hits.lock()
                .unwrap()
                .push(format!("{host} {}", req.uri().path()));
            next.run(req).await
        }
    };
    let app = routes.layer(middleware::from_fn(record));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// 一张 `width` × `height` 的 PNG。
pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbImage::new(width, height)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

/// 带 Open Graph 的网页。
pub(crate) fn og_page(title: &str, image: Option<&str>) -> String {
    let image = image
        .map(|src| format!(r#"<meta property="og:image" content="{src}">"#))
        .unwrap_or_default();
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
        <meta property="og:title" content="{title}">
        <meta property="og:description" content="About {title}">
        <meta property="og:site_name" content="Site">{image}
        </head><body>body</body></html>"#
    )
}
