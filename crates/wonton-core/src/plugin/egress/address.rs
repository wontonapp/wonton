//! 主人的代理地址（ADR-0023 的“代理地址从哪里来”）：`http://`、`https://`、`socks5://`，可以带用户名和密码。

use percent_encoding::percent_decode_str;
use url::Url;

use crate::l10n;
use crate::plugin::languages::Languages;

/// socks5 没有 URL 的默认端口。
const SOCKS_PORT: u16 = 1080;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
    Socks5,
}

/// 解析过的代理地址。密码只进出口容器和核心：`Debug` 和 [`Self::masked`] 都打码。
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyAddress {
    text: String,
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl ProxyAddress {
    pub fn parse(text: &str, langs: &Languages) -> Result<Self, String> {
        let url = Url::parse(text.trim()).map_err(|e| {
            l10n::text(
                langs,
                "plugin-settings-egress-address-invalid",
                &[("error", e.to_string().into())],
            )
        })?;
        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            "socks5" => Scheme::Socks5,
            other => {
                return Err(l10n::text(
                    langs,
                    "plugin-settings-egress-address-scheme",
                    &[("scheme", other.into())],
                ));
            }
        };
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| l10n::text(langs, "plugin-settings-egress-address-no-host", &[]))?
            .to_owned();
        let path = url.path();
        if !(path.is_empty() || path == "/") || url.query().is_some() || url.fragment().is_some() {
            return Err(l10n::text(
                langs,
                "plugin-settings-egress-address-no-path",
                &[],
            ));
        }
        let decode = |part: &str| -> Result<String, String> {
            percent_decode_str(part)
                .decode_utf8()
                .map(|text| text.into_owned())
                .map_err(|e| {
                    l10n::text(
                        langs,
                        "plugin-settings-egress-address-utf8",
                        &[("error", e.to_string().into())],
                    )
                })
        };
        let username = Some(url.username())
            .filter(|name| !name.is_empty())
            .map(decode)
            .transpose()?;
        let password = url.password().map(decode).transpose()?;
        Ok(Self {
            text: text.trim().to_owned(),
            scheme,
            host,
            port: url.port_or_known_default().unwrap_or(SOCKS_PORT),
            username,
            password,
        })
    }

    /// 主人写的原文，包括密码：只用来存下。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 给人看的地址，密码打码。
    pub fn masked(&self) -> String {
        let scheme = match self.scheme {
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Socks5 => "socks5",
        };
        let user = match (&self.username, &self.password) {
            (Some(name), Some(_)) => format!("{name}:***@"),
            (Some(name), None) => format!("{name}@"),
            (None, Some(_)) => ":***@".to_owned(),
            (None, None) => String::new(),
        };
        format!("{scheme}://{user}{}:{}", self.host, self.port)
    }

    /// `host:port`，IPv6 地址带方括号（[`Url::host_str`] 给的就带）。
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl std::fmt::Debug for ProxyAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.masked())
    }
}

#[cfg(test)]
fn langs() -> crate::plugin::languages::Languages {
    crate::plugin::languages::Languages::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_three_schemes_and_decodes_credentials() {
        let http = ProxyAddress::parse("http://host.docker.internal:7899", &langs()).unwrap();
        assert_eq!(http.scheme, Scheme::Http);
        assert_eq!(http.authority(), "host.docker.internal:7899");
        assert_eq!(http.username, None);

        let https = ProxyAddress::parse("https://proxy.example.com", &langs()).unwrap();
        assert_eq!((https.scheme, https.port), (Scheme::Https, 443));

        let socks = ProxyAddress::parse("socks5://u%40x:p%3Aw@[fd00::1]", &langs()).unwrap();
        assert_eq!((socks.scheme, socks.port), (Scheme::Socks5, 1080));
        assert_eq!(socks.authority(), "[fd00::1]:1080");
        assert_eq!(socks.username.as_deref(), Some("u@x"));
        assert_eq!(socks.password.as_deref(), Some("p:w"));
    }

    #[test]
    fn it_turns_down_what_is_not_a_proxy_address() {
        for text in [
            "",
            "proxy:7899",
            "ftp://proxy:21",
            "http://",
            "http://proxy:7899/path",
            "http://proxy:7899/?x=1",
            "http://proxy:7899/#x",
        ] {
            assert!(ProxyAddress::parse(text, &langs()).is_err(), "{text}");
        }
    }

    #[test]
    fn the_password_is_not_shown() {
        let address = ProxyAddress::parse("http://me:secret@proxy:7899", &langs()).unwrap();
        assert_eq!(address.masked(), "http://me:***@proxy:7899");
        assert!(!format!("{address:?}").contains("secret"));
        assert_eq!(address.text(), "http://me:secret@proxy:7899");
    }
}
