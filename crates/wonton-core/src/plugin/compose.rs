//! 核心接受的 Compose 子集。字段是白名单：不在这里的字段一律报错，而不是悄悄忽略。
//!
//! 安全相关的字段（端口、宿主网络、特权、Docker socket 等）一律拒绝，并给出原因；它们对应的高权限随权限确认
//! 流程逐个开放。开放了的也不收 compose 里的写法：显卡由核心按清单的 `gpu.services` 和宿主实际的显卡交给服务
//! （[`super::gpu`]），插件写不了设备路径。

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

/// 服务级别明确不支持的字段及原因。其余未知字段由 serde 的 `deny_unknown_fields` 拒绝。
pub const FORBIDDEN_SERVICE_KEYS: &[(&str, &str)] = &[
    ("ports", "插件不向宿主暴露端口，网页一律经核心网关访问"),
    ("networks", "插件网络由核心创建和管理"),
    ("container_name", "容器名由核心分配"),
    ("build", "插件只能使用已发布的镜像，不在宿主上构建"),
    (
        "env_file",
        "插件包只有一个 compose 文件，环境变量请写进 environment",
    ),
    ("extends", "插件包只有一个 compose 文件，不支持 extends"),
    ("links", "同一插件的服务在同一网络内，直接用服务名互访"),
    ("external_links", "插件之间默认互不可达"),
    ("volumes_from", "请用命名卷或应用数据目录共享数据"),
];

/// 顶层明确不支持的字段及原因。
pub const FORBIDDEN_TOP_KEYS: &[(&str, &str)] = &[
    ("networks", "插件网络由核心创建和管理"),
    ("secrets", "M0 尚不支持 secrets，密钥由核心注入"),
];

/// `network_mode` 接受 `none` 或声明权限后的 `host`，其余取值按原因拒绝。
pub fn network_mode_refusal(value: &str) -> &'static str {
    match value {
        "host" => "宿主网络属于高权限（network.host）",
        _ => "network_mode 只接受 none 或 host；插件网络由核心创建和管理",
    }
}

/// 顶层 `configs` 里明确不支持的字段及原因。
pub const FORBIDDEN_CONFIG_KEYS: &[(&str, &str)] = &[
    ("external", "配置只能来自插件包"),
    (
        "environment",
        "核心不把自己的环境变量传给插件，需要变量时用 content 插值",
    ),
    ("name", "核心不创建 Docker 配置对象，不需要 name"),
];

/// 字符串或数字、布尔：compose 允许在环境变量、标签等处写非字符串标量。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Scalar {
    pub fn as_string(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::Bool(b) => b.to_string(),
        }
    }
}

/// compose `user` 的几种写法（插值之后）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserSpec {
    /// 没写，或是 root：`root`、`0`、`0:0`（组也可以写 `root`）。
    Root,
    /// 数字形式的 `uid:gid`。
    Ids { uid: u32, gid: u32 },
    /// 只有数字 uid，组由镜像里的 `/etc/passwd` 决定。
    Uid(u32),
    /// 用户名或组名（root 以外），要看镜像里的 `/etc/passwd` 才知道是谁。
    Named(String),
}

impl UserSpec {
    pub fn parse(user: Option<&Scalar>) -> Self {
        let Some(text) = user.map(Scalar::as_string) else {
            return Self::Root;
        };
        let (uid, gid) = match text.split_once(':') {
            Some((uid, gid)) => (uid, Some(gid)),
            None => (text.as_str(), None),
        };
        let root = |part: &str| matches!(part, "root" | "0");
        if root(uid) && gid.is_none_or(root) {
            return Self::Root;
        }
        match (uid.parse(), gid.map(str::parse)) {
            (Ok(uid), Some(Ok(gid))) => Self::Ids { uid, gid },
            (Ok(uid), None) => Self::Uid(uid),
            _ => Self::Named(text),
        }
    }
}

/// `command`、`entrypoint`、`tmpfs` 等：一个字符串或字符串列表。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    One(String),
    Many(Vec<String>),
}

/// `environment`、`labels`：映射或 `KEY=VALUE` 列表。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum KeyValues {
    Map(BTreeMap<String, Option<Scalar>>),
    List(Vec<String>),
}

impl KeyValues {
    /// 展开成键值对。没有值的键（`KEY` 或 `KEY:`）在 compose 里表示“取宿主环境变量”，
    /// 核心不把自己的环境泄露给插件，因此报错。
    pub fn pairs(&self, field: &str) -> Result<Vec<(String, String)>, String> {
        let missing = |key: &str| format!("{field} 中的 `{key}` 没有值（核心不传递宿主环境变量）");
        match self {
            Self::Map(map) => map
                .iter()
                .map(|(k, v)| match v {
                    Some(v) => Ok((k.clone(), v.as_string())),
                    None => Err(missing(k)),
                })
                .collect(),
            Self::List(items) => items
                .iter()
                .map(|item| match item.split_once('=') {
                    Some((k, v)) => Ok((k.to_owned(), v.to_owned())),
                    None => Err(missing(item)),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum VolumeEntry {
    Short(String),
    Long(LongVolume),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeType {
    Bind,
    Volume,
    Tmpfs,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongVolume {
    #[serde(rename = "type")]
    pub kind: VolumeType,
    #[serde(default)]
    pub source: Option<String>,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Healthcheck {
    #[serde(default)]
    pub test: Option<StringOrList>,
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub start_period: Option<String>,
    #[serde(default)]
    pub start_interval: Option<String>,
    #[serde(default)]
    pub disable: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependCondition {
    #[default]
    ServiceStarted,
    ServiceHealthy,
    ServiceCompletedSuccessfully,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependSpec {
    #[serde(default)]
    pub condition: DependCondition,
    #[serde(default)]
    pub restart: Option<bool>,
    #[serde(default)]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum DependsOn {
    List(Vec<String>),
    Map(BTreeMap<String, DependSpec>),
}

impl DependsOn {
    pub fn entries(&self) -> Vec<(String, DependCondition)> {
        match self {
            Self::List(names) => names
                .iter()
                .map(|n| (n.clone(), DependCondition::ServiceStarted))
                .collect(),
            Self::Map(map) => map.iter().map(|(n, s)| (n.clone(), s.condition)).collect(),
        }
    }
}

/// 服务的 `configs`：短语法只写配置名，放到 `/<名字>`；长语法可以指定目标、属主和权限。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ConfigRef {
    Short(String),
    Long(LongConfigRef),
}

impl ConfigRef {
    pub fn long(&self) -> LongConfigRef {
        match self {
            Self::Short(source) => LongConfigRef {
                source: source.clone(),
                target: None,
                uid: None,
                gid: None,
                mode: None,
            },
            Self::Long(long) => long.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongConfigRef {
    pub source: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub uid: Option<Scalar>,
    #[serde(default)]
    pub gid: Option<Scalar>,
    #[serde(default)]
    pub mode: Option<Scalar>,
}

/// 只接受 `none`，其余取值在 [`super::package`] 里按原因拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    None,
    Host,
}
/// Compose 的固定设备映射。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum DeviceEntry {
    Short(String),
    Long(LongDevice),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongDevice {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub path_on_host: Option<String>,
    #[serde(default)]
    pub path_in_container: Option<String>,
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub cgroup_permissions: Option<String>,
}

impl DeviceEntry {
    pub fn paths(&self) -> Result<(String, String, String), String> {
        match self {
            Self::Short(text) => {
                let parts: Vec<&str> = text.split(':').collect();
                let (source, target, permissions) = match parts.as_slice() {
                    [source, target] => (*source, *target, "rwm"),
                    [source, target, permissions] => (*source, *target, *permissions),
                    _ => return Err(format!("设备映射 `{text}` 格式不合法")),
                };
                Ok((source.to_owned(), target.to_owned(), permissions.to_owned()))
            }
            Self::Long(device) => {
                let source = device
                    .source
                    .clone()
                    .or_else(|| device.path_on_host.clone())
                    .ok_or_else(|| "设备映射缺少 source/path_on_host".to_owned())?;
                let target = device
                    .target
                    .clone()
                    .or_else(|| device.path_in_container.clone())
                    .ok_or_else(|| "设备映射缺少 target/path_in_container".to_owned())?;
                let permissions = device
                    .permissions
                    .clone()
                    .or_else(|| device.cgroup_permissions.clone())
                    .unwrap_or_else(|| "rwm".to_owned());
                Ok((source, target, permissions))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullPolicy {
    Always,
    #[default]
    Missing,
    Never,
}

/// 一个 compose 服务。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub image: String,
    #[serde(default)]
    pub command: Option<StringOrList>,
    #[serde(default)]
    pub entrypoint: Option<StringOrList>,
    #[serde(default)]
    pub environment: Option<KeyValues>,
    #[serde(default)]
    pub volumes: Vec<VolumeEntry>,
    #[serde(default)]
    pub configs: Vec<ConfigRef>,
    #[serde(default)]
    pub restart: Option<String>,
    #[serde(default)]
    pub healthcheck: Option<Healthcheck>,
    #[serde(default)]
    pub depends_on: Option<DependsOn>,
    #[serde(default)]
    pub user: Option<Scalar>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub labels: Option<KeyValues>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub expose: Vec<Scalar>,
    #[serde(default)]
    pub init: Option<bool>,
    #[serde(default)]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub shm_size: Option<Scalar>,
    #[serde(default)]
    pub mem_limit: Option<Scalar>,
    #[serde(default)]
    pub mem_reservation: Option<Scalar>,
    #[serde(default)]
    pub cpus: Option<Scalar>,
    #[serde(default)]
    pub cpu_shares: Option<Scalar>,
    #[serde(default)]
    pub pids_limit: Option<Scalar>,
    #[serde(default)]
    pub tmpfs: Option<StringOrList>,
    #[serde(default)]
    pub stop_signal: Option<String>,
    #[serde(default)]
    pub stop_grace_period: Option<String>,
    #[serde(default)]
    pub pull_policy: PullPolicy,
    #[serde(default)]
    pub network_mode: Option<NetworkMode>,
    #[serde(default)]
    pub cap_add: Vec<String>,
    #[serde(default)]
    pub devices: Vec<DeviceEntry>,
    #[serde(default)]
    pub pid: Option<String>,
    #[serde(default)]
    pub ipc: Option<String>,
    #[serde(default)]
    pub security_opt: Vec<String>,
    #[serde(default)]
    pub privileged: Option<bool>,
}

/// 去掉对象里 `x-` 开头的扩展字段（compose 规范允许在任意层级出现）。
pub fn strip_extensions(value: &mut Value) {
    if let Value::Object(map) = value {
        map.retain(|key, _| !key.starts_with("x-"));
    }
}

/// 解析 compose 时长：`1h30m`、`10s`、`500ms`、`1m30.5s`。
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || format!("时长 `{text}` 不合法（示例：30s、1m30s、500ms）");
    let mut total = 0f64;
    let mut rest = text.trim();
    if rest.is_empty() {
        return Err(bad());
    }
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .ok_or_else(bad)?;
        let number: f64 = rest[..num_len].parse().map_err(|_| bad())?;
        rest = &rest[num_len..];
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 1e-3,
            "us" | "µs" => 1e-6,
            "ns" => 1e-9,
            _ => return Err(bad()),
        };
        total += number * factor;
        rest = &rest[unit_len..];
    }
    Ok(Duration::from_secs_f64(total))
}

/// 解析 compose 字节数：`64m`、`1g`、`512kb`、`1024`。
pub fn parse_bytes(value: &Scalar) -> Result<i64, String> {
    let text = value.as_string().to_ascii_lowercase();
    let bad = || format!("大小 `{text}` 不合法（示例：64m、1g）");
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let number: i64 = text[..split].parse().map_err(|_| bad())?;
    let factor: i64 = match &text[split..] {
        "" | "b" => 1,
        "k" | "kb" => 1 << 10,
        "m" | "mb" => 1 << 20,
        "g" | "gb" => 1 << 30,
        _ => return Err(bad()),
    };
    number.checked_mul(factor).ok_or_else(bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_sizes() {
        assert_eq!(parse_duration("1m30s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        for bad in ["", "10", "5x", "s"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_bytes(&Scalar::Str("64m".into())).unwrap(), 64 << 20);
        assert_eq!(parse_bytes(&Scalar::Int(1024)).unwrap(), 1024);
        assert!(parse_bytes(&Scalar::Str("1t".into())).is_err());
    }

    #[test]
    fn user_forms_are_told_apart() {
        let parse = |text: &str| UserSpec::parse(Some(&Scalar::Str(text.to_owned())));
        assert_eq!(UserSpec::parse(None), UserSpec::Root);
        for root in ["root", "0", "0:0", "root:root", "0:root"] {
            assert_eq!(parse(root), UserSpec::Root, "{root}");
        }
        assert_eq!(
            parse("65532:65532"),
            UserSpec::Ids {
                uid: 65532,
                gid: 65532
            }
        );
        assert_eq!(parse("0:1000"), UserSpec::Ids { uid: 0, gid: 1000 });
        assert_eq!(parse("1000"), UserSpec::Uid(1000));
        assert_eq!(
            UserSpec::parse(Some(&Scalar::Int(1000))),
            UserSpec::Uid(1000)
        );
        for named in ["nobody", "app:app", "1000:staff", "node:1000"] {
            assert_eq!(parse(named), UserSpec::Named(named.to_owned()), "{named}");
        }
    }

    #[test]
    fn key_values_without_value_are_rejected() {
        let list = KeyValues::List(vec!["A=1".into(), "B".into()]);
        assert!(list.pairs("environment").is_err());
        let map: KeyValues =
            serde_json::from_value(serde_json::json!({"A": 1, "B": true})).unwrap();
        assert_eq!(
            map.pairs("environment").unwrap(),
            vec![("A".into(), "1".into()), ("B".into(), "true".into())]
        );
    }
}
