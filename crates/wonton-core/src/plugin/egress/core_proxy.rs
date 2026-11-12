//! 核心自己访问外网（GitHub、官方索引、推送）用的代理（ADR-0023 的“核心和系统容器”）：直接连主人的代理，不经出口容器，也不读
//! `HTTPS_PROXY`。按请求取当前的地址：主人在 App 里改了之后不用重建客户端。

use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use url::{Host, Url};

use super::address::ProxyAddress;

#[derive(Clone, Default)]
pub struct CoreProxy(Arc<RwLock<Option<Url>>>);

impl CoreProxy {
    pub fn set(&self, address: Option<&ProxyAddress>) {
        let url = address.and_then(|address| Url::parse(address.text()).ok());
        *self.0.write().expect("核心代理锁中毒") = url;
    }

    /// 给 HTTP 客户端的设置。设了它，reqwest 就不再读环境变量。
    pub fn for_client(&self) -> reqwest::Proxy {
        let current = Arc::clone(&self.0);
        reqwest::Proxy::custom(move |target| {
            if is_local(target) {
                return None;
            }
            current.read().expect("核心代理锁中毒").clone()
        })
    }
}

/// 不经代理的目标：推送地址来自设备，可以是家里的 ntfy；主人的代理多半到不了也解析不了这些名字。
fn is_local(target: &Url) -> bool {
    let private = |ip: IpAddr| match ip {
        IpAddr::V4(ip) => {
            let shared = ip.octets()[0] == 100 && ip.octets()[1] & 0xc0 == 64;
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || shared
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    };
    match target.host() {
        Some(Host::Ipv4(ip)) => private(ip.into()),
        Some(Host::Ipv6(ip)) => private(ip.into()),
        Some(Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            !name.contains('.')
                || [".localhost", ".local", ".lan", ".internal"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// 只回答一次的代理，把请求头交回来。
    async fn one_request() -> (u16, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 4096];
            let read = stream.read(&mut buffer).await.unwrap();
            let reply = b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
            stream.write_all(reply).await.unwrap();
            String::from_utf8_lossy(&buffer[..read]).into_owned()
        });
        (port, task)
    }

    #[tokio::test]
    async fn the_address_set_later_applies_to_the_same_client_with_its_password() {
        let proxy = CoreProxy::default();
        let client = reqwest::Client::builder()
            .proxy(proxy.for_client())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (port, seen) = one_request().await;
        let target = "http://example.invalid/index";
        client.get(target).send().await.unwrap_err();

        let address = format!("http://me:s3cret@127.0.0.1:{port}");
        proxy.set(Some(&ProxyAddress::parse(&address).unwrap()));
        let reply = client.get(target).send().await.unwrap();
        assert_eq!(reply.status().as_u16(), 204);
        let head = seen.await.unwrap().to_ascii_lowercase();
        assert!(
            head.starts_with("get http://example.invalid/index"),
            "{head}"
        );
        // me:s3cret 的 base64。
        assert!(
            head.contains("proxy-authorization: basic bwu6cznjcmv0"),
            "{head}"
        );

        proxy.set(None);
        client.get(target).send().await.unwrap_err();
    }

    #[tokio::test]
    async fn local_targets_skip_the_proxy() {
        let proxy = CoreProxy::default();
        let unused = ProxyAddress::parse("http://127.0.0.1:1").unwrap();
        proxy.set(Some(&unused));
        let client = reqwest::Client::builder()
            .proxy(proxy.for_client())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (port, seen) = one_request().await;
        let url = format!("http://127.0.0.1:{port}/push");
        assert_eq!(
            client.get(&url).send().await.unwrap().status().as_u16(),
            204
        );
        assert!(seen.await.unwrap().starts_with("GET /push"));
    }

    #[test]
    fn what_counts_as_local() {
        for local in [
            "http://192.168.1.5/",
            "http://10.0.0.1/",
            "http://172.17.0.1/",
            "http://100.64.0.1/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://ntfy/",
            "http://ntfy.lan/",
            "http://hydra.wonton.internal/",
            "http://wonton.localhost/",
        ] {
            assert!(is_local(&Url::parse(local).unwrap()), "{local}");
        }
        for public in [
            "https://github.com/",
            "https://1.1.1.1/",
            "https://100.128.0.1/",
            "https://[2606:4700:4700::1111]/",
        ] {
            assert!(!is_local(&Url::parse(public).unwrap()), "{public}");
        }
    }
}
