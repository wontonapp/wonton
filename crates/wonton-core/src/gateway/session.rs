//! 网关会话。
//!
//! - 核心会话：在核心域名 `<base>` 上，主人登录后获得（M0 用主人令牌登录，配对上线后改为设备会话）。
//! - 插件会话：每个插件子域名各自一个，只在本域名有效。浏览器先到核心换一次性交接码，
//!   再到插件域名兑换，这样插件之间、插件与核心之间都不共享 Cookie。
//!
//! 会话只存在内存里：核心重启后需要重新登录。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hyper::HeaderMap;
use hyper::header::{COOKIE, HeaderValue};

use crate::secret::random_token;

pub const CORE_COOKIE: &str = "wonton_session";
pub const APP_COOKIE: &str = "wonton_app";

const RESERVED_PREFIX: &[u8] = b"wonton_";

pub const CORE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
pub const APP_TTL: Duration = Duration::from_secs(12 * 3600);
const CODE_TTL: Duration = Duration::from_secs(60);

struct Expiring<T> {
    value: T,
    expires: Instant,
}

#[derive(Clone, PartialEq, Eq)]
struct Bound {
    user: String,
    host: String,
}

#[derive(Default)]
struct Inner {
    core: HashMap<String, Expiring<String>>,
    app: HashMap<String, Expiring<Bound>>,
    codes: HashMap<String, Expiring<Bound>>,
}

#[derive(Default)]
pub struct SessionStore {
    inner: Mutex<Inner>,
}

fn live<T: Clone>(map: &mut HashMap<String, Expiring<T>>, key: &str) -> Option<T> {
    let now = Instant::now();
    map.retain(|_, v| v.expires > now);
    map.get(key).map(|v| v.value.clone())
}

impl SessionStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("会话表锁中毒")
    }

    pub fn create_core(&self, user: &str) -> String {
        let sid = random_token();
        let entry = Expiring {
            value: user.to_owned(),
            expires: Instant::now() + CORE_TTL,
        };
        self.lock().core.insert(sid.clone(), entry);
        sid
    }

    pub fn core_user(&self, sid: &str) -> Option<String> {
        live(&mut self.lock().core, sid)
    }

    /// 退出：删除核心会话，并作废该用户的所有插件会话和交接码。
    pub fn revoke_core(&self, sid: &str) {
        let mut inner = self.lock();
        if let Some(entry) = inner.core.remove(sid) {
            let user = entry.value;
            inner.app.retain(|_, v| v.value.user != user);
            inner.codes.retain(|_, v| v.value.user != user);
        }
    }

    /// 为某个插件域名签发一次性交接码，60 秒内有效。
    pub fn create_code(&self, user: &str, host: &str) -> String {
        let code = random_token();
        let entry = Expiring {
            value: Bound {
                user: user.to_owned(),
                host: host.to_owned(),
            },
            expires: Instant::now() + CODE_TTL,
        };
        self.lock().codes.insert(code.clone(), entry);
        code
    }

    /// 兑换交接码：只能用一次，且只能在签发时指定的域名上兑换。
    pub fn redeem_code(&self, code: &str, host: &str) -> Option<String> {
        let mut inner = self.lock();
        let bound = live(&mut inner.codes, code)?;
        inner.codes.remove(code);
        (bound.host == host).then_some(bound.user)
    }

    pub fn create_app(&self, user: &str, host: &str) -> String {
        let sid = random_token();
        let entry = Expiring {
            value: Bound {
                user: user.to_owned(),
                host: host.to_owned(),
            },
            expires: Instant::now() + APP_TTL,
        };
        self.lock().app.insert(sid.clone(), entry);
        sid
    }

    pub fn app_user(&self, sid: &str, host: &str) -> Option<String> {
        let bound = live(&mut self.lock().app, sid)?;
        (bound.host == host).then_some(bound.user)
    }
}

/// `wonton_` 开头的 Cookie 名归网关：插件设置不了（见 `isolation::sanitize_set_cookies`），也收不到。
/// 不分大小写，免得各处比较方式不一致。
pub fn is_reserved_cookie(name: &[u8]) -> bool {
    name.get(..RESERVED_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(RESERVED_PREFIX))
}

/// 请求 `Cookie` 头里的每一对 `名=值`。按字节解析：插件可以设置非 ASCII 的 Cookie，网关不能因此读不出
/// 自己的会话。
fn cookie_pairs(headers: &HeaderMap) -> impl Iterator<Item = (&[u8], &[u8])> {
    headers
        .get_all(COOKIE)
        .iter()
        .flat_map(|v| v.as_bytes().split(|&b| b == b';'))
        .filter_map(|pair| {
            let pair = pair.trim_ascii();
            let at = pair.iter().position(|&b| b == b'=')?;
            Some((pair[..at].trim_ascii(), pair[at + 1..].trim_ascii()))
        })
}

/// 请求里名为 `name` 的每一个 Cookie 的值。同名的可能不止一个：插件页面的脚本能给父域写同名 Cookie，
/// 还能用更长的路径排到真正的会话前面，所以调用方要逐个试，不能只看第一个。
pub fn cookies<'a>(headers: &'a HeaderMap, name: &'a str) -> impl Iterator<Item = &'a str> {
    cookie_pairs(headers)
        .filter(move |(key, _)| *key == name.as_bytes())
        .filter_map(|(_, value)| std::str::from_utf8(value).ok())
}

/// 从转发出去的请求里去掉名字满足 `strip` 的 Cookie，其余的原样保留。
pub fn strip_cookies(headers: &mut HeaderMap, strip: impl Fn(&[u8]) -> bool) {
    let mut kept = Vec::new();
    let pairs = headers
        .get_all(COOKIE)
        .iter()
        .flat_map(|v| v.as_bytes().split(|&b| b == b';'));
    for pair in pairs.map(<[u8]>::trim_ascii) {
        let name = pair.split(|&b| b == b'=').next().unwrap_or_default();
        if pair.is_empty() || strip(name.trim_ascii()) {
            continue;
        }
        if !kept.is_empty() {
            kept.extend_from_slice(b"; ");
        }
        kept.extend_from_slice(pair);
    }
    headers.remove(COOKIE);
    if !kept.is_empty()
        && let Ok(value) = HeaderValue::from_bytes(&kept)
    {
        headers.insert(COOKIE, value);
    }
}

pub fn set_cookie(name: &str, value: &str, max_age: Duration, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        max_age.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_codes_are_single_use_and_host_bound() {
        let store = SessionStore::default();
        let code = store.create_code("owner", "a.wonton.localhost");
        assert_eq!(store.redeem_code(&code, "b.wonton.localhost"), None);
        let code = store.create_code("owner", "a.wonton.localhost");
        assert_eq!(
            store.redeem_code(&code, "a.wonton.localhost"),
            Some("owner".into())
        );
        assert_eq!(store.redeem_code(&code, "a.wonton.localhost"), None);
    }

    #[test]
    fn app_sessions_are_host_bound_and_die_with_logout() {
        let store = SessionStore::default();
        let core = store.create_core("owner");
        let app = store.create_app("owner", "a.wonton.localhost");
        assert_eq!(
            store.app_user(&app, "a.wonton.localhost"),
            Some("owner".into())
        );
        assert_eq!(store.app_user(&app, "b.wonton.localhost"), None);
        store.revoke_core(&core);
        assert_eq!(store.core_user(&core), None);
        assert_eq!(store.app_user(&app, "a.wonton.localhost"), None);
    }

    #[test]
    fn gateway_cookies_never_reach_plugins() {
        let mut headers = HeaderMap::new();
        headers.append(
            COOKIE,
            "grafana_session=g; wonton_app=secret".parse().unwrap(),
        );
        headers.append(
            COOKIE,
            HeaderValue::from_bytes(
                b"wonton_session=core; name=\xe5\xbc\xa0; Wonton_X=1; wonton_app=2",
            )
            .unwrap(),
        );
        let apps: Vec<&str> = cookies(&headers, APP_COOKIE).collect();
        assert_eq!(
            apps,
            ["secret", "2"],
            "同名的每一个都要读出来，非 ASCII 的 Cookie 不影响"
        );
        strip_cookies(&mut headers, is_reserved_cookie);
        assert_eq!(
            headers.get(COOKIE).unwrap().as_bytes(),
            b"grafana_session=g; name=\xe5\xbc\xa0"
        );
        let mut only_ours = HeaderMap::new();
        only_ours.insert(COOKIE, "wonton_app=x".parse().unwrap());
        strip_cookies(&mut only_ours, is_reserved_cookie);
        assert!(only_ours.get(COOKIE).is_none());
    }
}
