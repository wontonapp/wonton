//! 要取的网址和它的地址：只认 http(s) 和四个端口，域名由核心自己解析，解析出的地址有一个不许去就整个拒绝（ADR-0065）。

use std::net::IpAddr;

use url::{Host, Url};
use wonton_protocol::LinkPreviewFailure;

use super::Failure;
use super::connect::Net;
use crate::net::special::{self, Kind};

/// 只允许这几个端口：网站用的就是它们，别的端口多半是内部服务。
const ALLOWED_PORTS: [u16; 4] = [80, 443, 8080, 8443];

/// 校验过的网址：http(s)、有主机、没有用户名密码、端口在允许的范围里。去掉了 `#` 后面的部分，它不影响取到的内容。
pub(super) fn parse(text: &str) -> Result<Url, Failure> {
    let url = Url::parse(text.trim())
        .map_err(|e| Failure::new(LinkPreviewFailure::InvalidUrl, e.to_string()))?;
    check(url)
}

/// 跳转的目标按同样的规矩检查。
pub(super) fn check(mut url: Url) -> Result<Url, Failure> {
    let invalid = |detail: String| Failure::new(LinkPreviewFailure::InvalidUrl, detail);
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid(url.scheme().to_owned()));
    }
    if url.host().is_none() {
        return Err(invalid(url.to_string()));
    }
    // 带用户名密码的网址会把它们发给对方，链接预览用不到。
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(url.host_str().unwrap_or_default().to_owned()));
    }
    let port = url.port_or_known_default().unwrap_or(0);
    if !ALLOWED_PORTS.contains(&port) {
        return Err(invalid(port.to_string()));
    }
    url.set_fragment(None);
    Ok(url)
}

/// 网址的主机现在对应的地址，全部检查过；有一个不许去就整个拒绝：连哪个由我们选，但留着内网地址的域名本身就可疑。
pub(super) async fn addresses(net: &Net, url: &Url) -> Result<Vec<IpAddr>, Failure> {
    let port = url.port_or_known_default().unwrap_or(0);
    let found = match url.host() {
        Some(Host::Ipv4(ip)) => vec![IpAddr::V4(ip)],
        Some(Host::Ipv6(ip)) => vec![IpAddr::V6(ip)],
        Some(Host::Domain(name)) => (net.resolve)(name.to_owned(), port)
            .await
            .map_err(|e| Failure::new(LinkPreviewFailure::UpstreamError, e.to_string()))?,
        None => {
            return Err(Failure::new(
                LinkPreviewFailure::InvalidUrl,
                url.to_string(),
            ));
        }
    };
    if found.is_empty() {
        let host = url.host_str().unwrap_or_default().to_owned();
        return Err(Failure::new(LinkPreviewFailure::UpstreamError, host));
    }
    if let Some(ip) = found.iter().copied().find(|ip| blocked(*ip)) {
        return Err(Failure::new(
            LinkPreviewFailure::BlockedAddress,
            ip.to_string(),
        ));
    }
    Ok(found)
}

/// 不许去的地址：ADR-0065 列的几类，加上 0/8（Linux 上连它会连到本机）和 240/4（含广播）。
/// IPv4 映射、兼容和 NAT64 写法按里面的 IPv4 判断（见 [`special::classify`]）。
///
/// 文档、基准测试（198.18/15）和 IETF 协议段放行：它们在公网上不路由，连不上就是对方站点出错；而 Clash 这类代理的
/// fake-ip 模式把所有域名解析到 198.18/15，挡掉它会让这样配的家里取不到任何预览。
pub(super) fn blocked(ip: IpAddr) -> bool {
    match special::classify(ip) {
        None | Some(Kind::Documentation | Kind::Benchmarking | Kind::IetfProtocol) => false,
        Some(
            Kind::Unspecified
            | Kind::ThisNetwork
            | Kind::Loopback
            | Kind::Private
            | Kind::Cgnat
            | Kind::LinkLocal
            | Kind::Ula
            | Kind::Multicast
            | Kind::Reserved,
        ) => true,
    }
}
