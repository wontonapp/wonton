//! 推送登记表：每台已配对设备一份推送订阅，存在 `core/push.json`，启动时读回，改动时整份重写。
//!
//! 认证密钥（`auth`）加上推送地址就能给设备伪造推送，文件权限 0600；`Debug` 不输出它们。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use web_push_native::p256::PublicKey;

use super::PushError;
use crate::secret::random_token;

/// 推送地址的长度上限：RFC 8030 没有规定，常见的推送服务都在几百字节内。
const MAX_ENDPOINT_LEN: usize = 2048;

/// 校验过的推送订阅（W3C Push API 的 `PushSubscription`）。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// 推送地址，http 或 https。
    pub endpoint: String,
    /// 设备的 P-256 公钥，未压缩的点（65 字节），base64url。
    pub p256dh: String,
    /// 认证密钥（16 字节），base64url。
    pub auth: String,
}

impl Subscription {
    /// 校验推送地址和两个密钥的格式；公钥是否在曲线上由加密时检查。
    pub fn new(endpoint: &str, p256dh: &str, auth: &str) -> Result<Self, PushError> {
        let url = url::Url::parse(endpoint)
            .map_err(|e| PushError::Invalid(format!("推送地址不是合法的 URL：{e}")))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(PushError::Invalid(
                "推送地址必须是 http 或 https 地址".to_owned(),
            ));
        }
        if endpoint.len() > MAX_ENDPOINT_LEN {
            return Err(PushError::Invalid(format!(
                "推送地址超过 {MAX_ENDPOINT_LEN} 字节"
            )));
        }
        let key = decode("p256dh", p256dh)?;
        if key.len() != 65 || key[0] != 0x04 {
            return Err(PushError::Invalid(
                "p256dh 应为 65 字节的未压缩 P-256 公钥".to_owned(),
            ));
        }
        if PublicKey::from_sec1_bytes(&key).is_err() {
            return Err(PushError::Invalid(
                "p256dh 不是 P-256 曲线上的点".to_owned(),
            ));
        }
        if decode("auth", auth)?.len() != 16 {
            return Err(PushError::Invalid("auth 应为 16 字节".to_owned()));
        }
        Ok(Self {
            endpoint: endpoint.to_owned(),
            p256dh: URL_SAFE_NO_PAD.encode(key),
            auth: auth.trim_end_matches('=').to_owned(),
        })
    }

    pub fn public_key(&self) -> Vec<u8> {
        URL_SAFE_NO_PAD
            .decode(&self.p256dh)
            .expect("登记时校验过 p256dh")
    }

    pub fn auth_secret(&self) -> Vec<u8> {
        URL_SAFE_NO_PAD
            .decode(&self.auth)
            .expect("登记时校验过 auth")
    }

    /// 推送地址的主机名，日志里只写它：地址的路径（比如 ntfy 的主题）知道了就能给设备发推送。
    pub fn endpoint_host(&self) -> String {
        url::Url::parse(&self.endpoint)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default()
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("endpoint_host", &self.endpoint_host())
            .finish_non_exhaustive()
    }
}

/// base64url，带不带 `=` 填充都接受。
fn decode(name: &str, value: &str) -> Result<Vec<u8>, PushError> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map_err(|e| PushError::Invalid(format!("{name} 不是 base64url：{e}")))
}

/// 一台设备的登记。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub device: EndpointId,
    /// Tuwunel 的 pusher 用它找到这台设备；256 位随机数，兼作推送代理的凭据。
    pub pushkey: String,
    #[serde(flatten)]
    pub subscription: Subscription,
}

impl std::fmt::Debug for Registration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registration")
            .field("device", &format_args!("{}", self.device.fmt_short()))
            .field("subscription", &self.subscription)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    registrations: Vec<Registration>,
}

pub struct Registrations {
    path: PathBuf,
    /// 锁同时串行化写文件。
    by_device: Mutex<BTreeMap<EndpointId, Registration>>,
}

impl Registrations {
    /// 读取登记表；文件不存在时从空表开始，损坏时报错，不当成空表。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PushError> {
        let path = path.into();
        let by_device = match fs::read_to_string(&path) {
            Ok(text) => {
                let stored: Stored =
                    serde_json::from_str(&text).map_err(|e| PushError::Corrupt {
                        path: path.clone(),
                        message: e.to_string(),
                    })?;
                stored
                    .registrations
                    .into_iter()
                    .map(|registration| (registration.device, registration))
                    .collect()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(source) => return Err(PushError::Io { path, source }),
        };
        Ok(Self {
            path,
            by_device: Mutex::new(by_device),
        })
    }

    /// 登记或更新一台设备的订阅。已有登记时沿用 pushkey，Tuwunel 上的 pusher 不用重新注册。
    pub fn upsert(
        &self,
        device: EndpointId,
        subscription: Subscription,
    ) -> Result<Registration, PushError> {
        let mut by_device = self.lock();
        let previous = by_device.get(&device).cloned();
        let registration = Registration {
            device,
            pushkey: previous
                .as_ref()
                .map_or_else(random_token, |previous| previous.pushkey.clone()),
            subscription,
        };
        by_device.insert(device, registration.clone());
        if let Err(e) = self.save(&by_device) {
            match previous {
                Some(previous) => by_device.insert(device, previous),
                None => by_device.remove(&device),
            };
            return Err(e);
        }
        Ok(registration)
    }

    /// 删除一台设备的登记。返回它之前是否登记过。
    pub fn remove(&self, device: &EndpointId) -> Result<bool, PushError> {
        let mut by_device = self.lock();
        let Some(previous) = by_device.remove(device) else {
            return Ok(false);
        };
        if let Err(e) = self.save(&by_device) {
            by_device.insert(*device, previous);
            return Err(e);
        }
        Ok(true)
    }

    /// 按 pushkey 找登记。pushkey 是凭据，逐个按常数时间比较。
    pub fn find(&self, pushkey: &str) -> Option<Registration> {
        self.lock()
            .values()
            .find(|registration| crate::secret::constant_time_eq(&registration.pushkey, pushkey))
            .cloned()
    }

    pub fn get(&self, device: &EndpointId) -> Option<Registration> {
        self.lock().get(device).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<EndpointId, Registration>> {
        self.by_device.lock().expect("推送登记表锁中毒")
    }

    /// 先写临时文件再改名，避免写到一半断电留下损坏的登记表。
    fn save(&self, by_device: &BTreeMap<EndpointId, Registration>) -> Result<(), PushError> {
        let io = |source| PushError::Io {
            path: self.path.clone(),
            source,
        };
        let stored = Stored {
            registrations: by_device.values().cloned().collect(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(|e| PushError::Corrupt {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&tmp, &self.path).map_err(io)
    }
}
