use iroh::RelayUrl;
use url::Url;

use super::relay::{Relay, RelayView};

/// 给界面、发现文件和日志用的规范化文字：根路径保留 `/`，非根路径去掉尾斜杠（`/relay/` → `/relay`）。
///
/// `Url` 序列化（以及 iroh 1.5 的 `RelayUrl`）会给非根路径补尾斜杠；配对码、连接状态和设置页
/// 统一成这里的一种写法才互相相等。
pub fn relay_url_text(url: &RelayUrl) -> String {
    url_text(&url.clone().into())
}

/// 报错时打的地址：去掉 userinfo，令牌不进错误串、日志和配对页。
pub(super) fn wire_url_text(url: &RelayUrl) -> String {
    let mut url: Url = url.clone().into();
    url.set_username("").ok();
    url.set_password(None).ok();
    url_text(&url)
}

/// 同 [`relay_url_text`]，参数是已经去过 userinfo/查询/片段的 [`Url`]。
///
/// 只看路径部分的斜杠，`?`/`#` 前面也照样去掉（`/relay/?x=1` → `/relay?x=1`）；根路径的 `/` 保留。
pub fn url_text(url: &Url) -> String {
    let mut text = url.as_str().to_owned();
    let cut = text.find(['?', '#']).unwrap_or(text.len());
    if cut > 0 && text.as_bytes()[cut - 1] == b'/' && url.path() != "/" {
        text.remove(cut - 1);
    }
    text
}

/// 中继域名不带 FQDN 尾点，使配对码、中继表和连接状态使用同一个 key（#200）。
///
/// iroh 内置中继带尾点，webpki 允许它作为 TLS 参照名；用户报告在其网络中去掉尾点才能配对。
/// 在配置和连接信息入口统一写法，不改 TLS 校验或信任根。
pub fn canonical_url(url: RelayUrl) -> RelayUrl {
    let Some(domain) = url.domain() else {
        return url;
    };
    let host = domain.trim_end_matches('.');
    if host == domain || host.is_empty() {
        return url;
    }
    let host = host.to_owned();
    let mut inner = Url::from(url);
    inner
        .set_host(Some(&host))
        .expect("removing the DNS root dot preserves a valid host");
    RelayUrl::from(inner)
}

/// [`RelayView`] 的地址规范成一种写法（[`canonical_url`]）。
pub fn canonical_view(view: RelayView) -> RelayView {
    RelayView {
        url: canonical_url(view.url),
        ..view
    }
}

/// [`Relay`] 的地址规范成一种写法（[`canonical_url`]）。
pub fn canonical_relay(relay: Relay) -> Relay {
    Relay {
        url: canonical_url(relay.url),
        ..relay
    }
}
