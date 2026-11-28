use iroh::RelayUrl;
use percent_encoding::percent_decode_str;
use url::Url;

use super::relay::{Relay, RelayToken};
use super::url::canonical_url;

/// 报错时打的地址：userinfo 里的令牌换成 `<redacted>`，令牌不进日志。
pub(super) fn redact_item(item: &str) -> String {
    let Some((scheme, rest)) = item.split_once("://") else {
        return item.to_owned();
    };
    // userinfo 只在 authority 里；query 里的 `@` 不算。
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = match authority.split_once('@') {
        Some((_, host)) => format!("<redacted>@{host}"),
        None => authority.to_owned(),
    };
    format!("{scheme}://{authority}{tail}")
}

/// 解析一项中继地址：`https://TOKEN@relay.example.org#7843`。令牌取 userinfo 的用户名（按百分号编码
/// 解码），解析后从地址里去掉；`#` 后面是 QUIC 端口，空着用默认端口。
pub fn relay_from_item(item: &str, tier: u8) -> Result<Relay, String> {
    let mut url =
        Url::parse(item).map_err(|e| format!("{}：不是合法的地址：{e}", redact_item(item)))?;
    if url.scheme() != "https" {
        return Err(format!("{}：中继地址要用 https", redact_item(item)));
    }
    let quic_port = match url.fragment() {
        None => None,
        Some(fragment) => {
            let port = fragment
                .parse::<u16>()
                .map_err(|_| format!("{}：`#` 后面要写端口号，比如 #7843", redact_item(item)))?;
            if port == 0 {
                return Err(format!("{}：QUIC 端口不能是 0", redact_item(item)));
            }
            url.set_fragment(None);
            Some(port)
        }
    };
    // `https://@host`：写了 userinfo 却是空的，多半是令牌丢了，报出来而不是当没写。
    // 只看 authority 段；query 里的 `@` 不是 userinfo。
    let after_scheme = item.split_once("://").map_or(item, |(_, rest)| rest);
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if authority.contains('@') && url.username().is_empty() && url.password().is_none() {
        return Err(format!("{}：令牌不能是空的", redact_item(item)));
    }
    let token = match (url.username(), url.password()) {
        ("", None) => None,
        (_, Some(_)) => {
            return Err(format!(
                "{}：令牌写在用户名里，不能有密码",
                redact_item(item)
            ));
        }
        (name, None) => {
            let decoded = percent_decode_str(name)
                .decode_utf8()
                .map_err(|_| format!("{}：令牌不是合法的 UTF-8", redact_item(item)))?;
            if decoded.is_empty() {
                return Err(format!("{}：令牌不能是空的", redact_item(item)));
            }
            Some(RelayToken::new(decoded.into_owned()))
        }
    };
    url.set_username("")
        .map_err(|_| format!("{}：去不掉 userinfo", redact_item(item)))?;
    let relay = Relay {
        url: canonical_url(RelayUrl::from(url)),
        token,
        quic_port,
        tier,
    };
    relay.validate()?;
    Ok(relay)
}
