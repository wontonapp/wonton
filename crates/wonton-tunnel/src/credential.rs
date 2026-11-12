//! 本地代理怎样认调用方。
//!
//! 本地代理监听回环地址，同一台设备上的其他 App 也能连上；服务端只认设备的配对身份，经代理来的请求
//! 都算这台设备的。所以 App 用的代理必须校验调用方：每次启动随机生成一份凭据，只交给本 App 的 WebView
//! 和 HTTP 客户端，其他 App 拿不到，也就借用不了配对身份（ADR-0004）。

use std::fmt;

use base64::{Engine, engine::general_purpose::STANDARD};

/// 本地代理接受哪些调用方、转发哪些地址。
#[derive(Debug, Clone)]
pub enum ProxyAccess {
    /// HTTP 正向代理，只接受带这份凭据的连接（`Proxy-Authorization: Basic`），支持 `CONNECT` 和绝对
    /// 形式的请求行。App 的 WebView 和 HTTP 客户端用这种。
    Credential {
        credential: ProxyCredential,
        /// 家里服务器的基础域名，例如 `wonton.localhost`。只转发它和它的子域名；别的地址（CDN、字体等
        /// 外部资源）回 403（`foreign-host`），不进隧道，别的浏览流量不会被送回家。
        domain: String,
    },
    /// 不校验调用方，把每条连接原样转发（反向代理形态，把代理当成服务端访问）。同机的任何进程都能借用
    /// 本设备的配对身份，只给开发工具和测试用。
    Open,
}

/// 本地代理的会话凭据：HTTP 代理认证的用户名和密码。`Debug` 输出不含密码。
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyCredential {
    username: String,
    password: String,
}

impl ProxyCredential {
    /// 随机生成一份凭据：用户名固定为 `wonton`，密码是 256 位随机数的十六进制。
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
        Self {
            username: "wonton".to_owned(),
            password: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    /// `Proxy-Authorization` 头的值是否是这份凭据（`Basic` 方案，大小写不敏感）。逐字节比较全部内容，
    /// 不按前缀提前返回。
    pub(crate) fn matches(&self, header: Option<&[u8]>) -> bool {
        let Some(value) = header.and_then(|value| std::str::from_utf8(value).ok()) else {
            return false;
        };
        let Some((scheme, encoded)) = value.trim().split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("basic") {
            return false;
        }
        let Ok(decoded) = STANDARD.decode(encoded.trim()) else {
            return false;
        };
        let expected = format!("{}:{}", self.username, self.password);
        decoded.len() == expected.len()
            && decoded
                .iter()
                .zip(expected.as_bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

impl fmt::Debug for ProxyCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyCredential")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(user_pass: &str) -> Vec<u8> {
        format!("Basic {}", STANDARD.encode(user_pass)).into_bytes()
    }

    #[test]
    fn only_the_exact_credential_matches() {
        let credential = ProxyCredential::generate();
        let right = format!("wonton:{}", credential.password());
        assert!(credential.matches(Some(&basic(&right))));
        let lowercase = format!("basic {}", STANDARD.encode(&right));
        assert!(credential.matches(Some(lowercase.as_bytes())));
        for wrong in [
            basic("wonton:"),
            basic(&format!("other:{}", credential.password())),
            basic(&right[..right.len() - 1]),
            format!("Bearer {}", credential.password()).into_bytes(),
            b"Basic not-base64!".to_vec(),
        ] {
            assert!(!credential.matches(Some(&wrong)));
        }
        assert!(!credential.matches(None));
        assert_ne!(credential, ProxyCredential::generate());
        assert!(!format!("{credential:?}").contains(credential.password()));
    }
}
