//! 链接预览测试的假世界：假 DNS 把域名指到给定的地址，拨号把它们都接到本机的假站点，时钟手动拨。
//! 不连真实网络，也不依赖 Docker。

use std::collections::HashMap;
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::middleware::{self, Next};
use tokio::net::{TcpListener, TcpStream};

use super::{Clock, Limits, LinkPreviews, Net};
use crate::plugin::egress::core_proxy::CoreProxy;

/// 假站点对外的“公网”地址（文档网段，预览不拦它）。
pub(crate) const SITE_IP: &str = "203.0.113.10";

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
}

/// 只认 `site.test` 和 `www.site.test` 的世界。
pub(crate) async fn world(routes: Router) -> (LinkPreviews, Handles) {
    world_with(routes, &[], Limits::default(), None).await
}

/// 假的 https 站点：地址和信任它的证书的 TLS 设置。
pub(crate) type TlsSite = (SocketAddr, Arc<rustls::ClientConfig>);

/// `dns` 补充或覆盖域名的地址；`tls_site` 给了的话，443 和 8443 端口拨到它。
pub(crate) async fn world_with(
    routes: Router,
    dns: &[(&str, &[&str])],
    limits: Limits,
    tls_site: Option<TlsSite>,
) -> (LinkPreviews, Handles) {
    let tls = tls_site.as_ref().map(|(_, config)| config.clone());
    let tls_site = tls_site.map(|(addr, _)| addr);
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
    let proxy = CoreProxy::default();
    let net = Net {
        resolve: Arc::new(move |name, _port| {
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
        tls,
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
        _dir: dir,
    };
    (previews, handles)
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
