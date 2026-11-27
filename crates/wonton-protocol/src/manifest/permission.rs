use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::permission_table::{
    HIGHEST_RISK_NOTICE, Notice, PermissionSpec, RiskLevel, RiskRule, permission_table,
};
use super::{ManifestError, is_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Read,
    Write,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Access {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            _ => Err(()),
        }
    }
}

// 改这张表（增删行、改写法或语义）要带 ADR（ADR-0128）。行的顺序就是 `Permission` 的 `Ord` 顺序，
// `optional_permissions` 这类 BTreeMap 按它遍历，不要随手调整。
permission_table! {
    StorageShare { name: String, access: Access } = "storage.share.{name}:{access}" {
        parse: "storage.share." => parse_storage_share, example: "storage.share.photos:read",
        risk: Normal, optional: false, env: None,
        text: "permission-storage-share", notice: Silent, supported: true,
    },
    StorageHome { access: Access } = "storage.home:{access}" {
        parse: "storage.home:" => parse_storage_home, example: "storage.home:write",
        risk: Normal, optional: false, env: None,
        text: "permission-storage-home", notice: Silent, supported: true,
    },
    Models = "models" {
        risk: Normal, optional: false, env: None,
        text: "permission-models", notice: Silent, supported: true,
    },
    Tools = "tools" {
        risk: Normal, optional: false, env: None,
        text: "permission-tools", notice: Silent, supported: true,
    },
    // 只读，只给状态，不算高权限（规范“插件健康探测”）。
    PluginsHealth = "plugins.health" {
        risk: Normal, optional: false, env: None,
        text: "permission-plugins-health", notice: Silent, supported: true,
    },
    MatrixBot = "matrix.bot" {
        risk: Normal, optional: false, env: None,
        text: "permission-matrix-bot", notice: Silent, supported: true,
    },
    Notify = "notify" {
        risk: Normal, optional: false, env: None,
        text: "permission-notify", notice: Silent, supported: true,
    },
    OwnerIdentity = "owner.identity" {
        risk: Normal, optional: false, env: None,
        text: "permission-owner-identity", notice: Silent, supported: true,
    },
    NetworkEgress = "network.egress" {
        risk: Normal, optional: false, env: None,
        text: "permission-network-egress", notice: Silent, supported: true,
    },
    EgressProxy = "egress.proxy" {
        risk: High, optional: false, env: None,
        text: "permission-egress-proxy", notice: Text("permission-notice-egress-proxy"),
        supported: true,
    },
    NetworkLan = "network.lan" {
        risk: High, optional: false, env: None,
        text: "permission-network-lan", notice: Text("permission-notice-network-lan"),
        supported: true,
    },
    // 不算高权限：装好不生效，等主人打开开关（ADR-0021）。
    NetworkClients = "network.clients" {
        risk: Normal, optional: false, env: None,
        text: "permission-network-clients", notice: Silent, supported: true,
    },
    NetworkHost = "network.host" {
        risk: VeryHigh, optional: false, env: None,
        text: "permission-network-host", notice: Text("permission-notice-network-host"),
        supported: true,
    },
    Capability { name: String } = "cap.{name}" {
        parse: "cap." => parse_capability, example: "cap.NET_ADMIN",
        risk: ByCapability, optional: false, env: None,
        text: "permission-capability", notice: HighestRisk, supported: true,
    },
    HostPid = "host.pid" {
        risk: Highest, optional: false, env: None,
        text: "permission-host-pid", notice: HighestRisk, supported: true,
    },
    HostIpc = "host.ipc" {
        risk: Highest, optional: false, env: None,
        text: "permission-host-ipc", notice: HighestRisk, supported: true,
    },
    SecurityOpt = "security.opt" {
        risk: Highest, optional: false, env: None,
        text: "permission-security-opt", notice: HighestRisk, supported: true,
    },
    Privileged = "privileged" {
        risk: Highest, optional: false, env: None,
        text: "permission-privileged", notice: HighestRisk, supported: true,
    },
    DevicesGpu = "devices.gpu" {
        risk: High, optional: true, env: Some("WONTON_PERMISSION_DEVICES_GPU"),
        text: "permission-devices-gpu", notice: Silent, supported: true,
    },
    /// 宿主串口设备，路径由主人选（ADR-0036）。
    DevicesSerial = "devices.serial" {
        risk: High, optional: true, env: Some("WONTON_PERMISSION_DEVICES_SERIAL"),
        text: "permission-devices-serial", notice: Text("permission-notice-devices"),
        supported: true,
    },
    /// 宿主 USB 设备，路径由主人选（ADR-0036）。
    DevicesUsb = "devices.usb" {
        risk: High, optional: true, env: Some("WONTON_PERMISSION_DEVICES_USB"),
        text: "permission-devices-usb", notice: Text("permission-notice-devices-usb"),
        supported: true,
    },
    // 硬盘能读到整盘上的数据，还带着能发写命令的 SYS_RAWIO。清单这一层放它过可选的名单，由核心的设备校验
    // 拒绝并说清为什么不能可选，所以没有注入变量。
    DevicesDisk = "devices.disk" {
        risk: VeryHigh, optional: true, env: None,
        text: "permission-devices-disk", notice: Text("permission-notice-devices-disk"),
        supported: true,
    },
    Device { path: String } = "devices.{path}" {
        parse: "devices." => parse_device, example: "devices./dev/net/tun",
        risk: High, optional: false, env: None,
        text: "permission-device", notice: Text("permission-notice-device"), supported: true,
    },
    DockerSocket = "docker.socket" {
        risk: Highest, optional: true, env: Some("WONTON_PERMISSION_DOCKER_SOCKET"),
        text: "permission-docker-socket", notice: HighestRisk, supported: true,
    },
}

fn parse_storage_share(rest: &str) -> Option<Permission> {
    let (name, access) = rest.split_once(':')?;
    let access = access.parse().ok()?;
    is_label(name).then(|| Permission::StorageShare {
        name: name.to_owned(),
        access,
    })
}

fn parse_storage_home(rest: &str) -> Option<Permission> {
    Some(Permission::StorageHome {
        access: rest.parse().ok()?,
    })
}

fn parse_capability(rest: &str) -> Option<Permission> {
    let name = rest.to_ascii_uppercase();
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    valid.then_some(Permission::Capability { name })
}

fn parse_device(rest: &str) -> Option<Permission> {
    rest.starts_with('/').then(|| Permission::Device {
        path: rest.to_owned(),
    })
}

/// `cap_add` 的风险级别；未列出的 Linux capability 一律按最高级。
pub fn capability_risk(name: &str) -> RiskLevel {
    match name.trim().to_ascii_uppercase().as_str() {
        "NET_ADMIN" | "NET_RAW" | "NET_BIND_SERVICE" | "SYS_TIME" | "SYS_NICE" | "IPC_LOCK" => {
            RiskLevel::VeryHigh
        }
        _ => RiskLevel::Highest,
    }
}

impl PermissionKind {
    pub fn spec(self) -> &'static PermissionSpec {
        &PERMISSIONS[self as usize]
    }
}

impl Permission {
    pub fn spec(&self) -> &'static PermissionSpec {
        self.kind().spec()
    }

    /// 风险级别，确认页、Bot 的卡片和清单限制共用；普通权限为空。
    pub fn risk(&self) -> Option<RiskLevel> {
        match self.spec().risk {
            RiskRule::Normal => None,
            RiskRule::High => Some(RiskLevel::High),
            RiskRule::VeryHigh => Some(RiskLevel::VeryHigh),
            RiskRule::Highest => Some(RiskLevel::Highest),
            RiskRule::ByCapability => Some(match self {
                Self::Capability { name } => capability_risk(name),
                _ => RiskLevel::Highest,
            }),
        }
    }

    /// 高权限在安装确认时单独醒目展示。
    pub fn is_high(&self) -> bool {
        self.risk().is_some()
    }

    /// 确认页在说明下面多写的那句提示的消息键；没有时为空。
    pub fn notice(&self) -> Option<&'static str> {
        match self.spec().notice {
            Notice::Silent => None,
            Notice::Text(key) => Some(key),
            Notice::HighestRisk => {
                (self.risk() == Some(RiskLevel::Highest)).then_some(HIGHEST_RISK_NOTICE)
            }
        }
    }
}

impl FromStr for Permission {
    type Err = ManifestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| ManifestError::UnknownPermission(value.to_owned()))
    }
}

impl TryFrom<String> for Permission {
    type Error = ManifestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Permission> for String {
    fn from(permission: Permission) -> Self {
        permission.to_string()
    }
}

#[cfg(test)]
#[path = "permission_tests.rs"]
mod tests;
