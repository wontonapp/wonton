//! 核心接受的 Compose 子集。字段是白名单：不在这里的字段一律报错，而不是悄悄忽略。
//!
//! 安全相关的字段（端口、宿主网络、特权、设备、Docker socket 等）在 M0 一律拒绝，并给出原因；
//! 它们对应的高权限在后续里程碑随权限确认流程一起开放。

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

/// 服务级别明确不支持的字段及原因。其余未知字段由 serde 的 `deny_unknown_fields` 拒绝。
pub const FORBIDDEN_SERVICE_KEYS: &[(&str, &str)] = &[
    ("ports", "插件不向宿主暴露端口，网页一律经核心网关访问"),
    (
        "network_mode",
        "宿主网络属于高权限（network.host），M0 尚未开放",
    ),
    ("networks", "插件网络由核心创建和管理"),
    ("privileged", "特权模式属于高权限，M0 尚未开放"),
    ("devices", "设备直通属于高权限（devices.*），M0 尚未开放"),
    ("cap_add", "增加内核能力属于高权限，M0 尚未开放"),
    ("security_opt", "修改安全选项属于高权限，M0 尚未开放"),
    ("pid", "共享宿主进程空间属于高权限"),
    ("ipc", "共享宿主 IPC 属于高权限"),
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
    (
        "configs",
        "M0 尚不支持 configs，请把配置写进环境变量或应用数据目录",
    ),
    ("secrets", "M0 尚不支持 secrets，密钥由核心注入"),
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
    pub tmpfs: Option<StringOrList>,
    #[serde(default)]
    pub stop_signal: Option<String>,
    #[serde(default)]
    pub stop_grace_period: Option<String>,
    #[serde(default)]
    pub pull_policy: PullPolicy,
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
