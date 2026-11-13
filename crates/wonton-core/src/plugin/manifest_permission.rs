//! 清单 `x-wonton.permissions` 的取值：权限名字的解析、显示和高权限表。
//!
//! 从 [`super::manifest`] 拆出来，那边的文件快到行数上限。权限的说明、风险级别和提示在
//! [`super::views_permissions`]。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::manifest::{ManifestError, is_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Read,
    Write,
}

/// 清单里声明、安装时由主人确认的权限。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Permission {
    StorageShare {
        name: String,
        access: Access,
    },
    StorageHome {
        access: Access,
    },
    Models,
    Tools,
    // 只读，只给状态，不算高权限（规范“插件健康探测”）。
    PluginsHealth,
    MatrixBot,
    Notify,
    OwnerIdentity,
    NetworkEgress,
    EgressProxy,
    NetworkLan,
    // 不算高权限：装好不生效，等主人打开开关（ADR-0021）。
    NetworkClients,
    NetworkHost,
    Capability {
        name: String,
    },
    HostPid,
    HostIpc,
    SecurityOpt,
    Privileged,
    DevicesGpu,
    /// 宿主串口设备，路径由主人选（ADR-0036）。
    DevicesSerial,
    /// 宿主 USB 设备，路径由主人选（ADR-0036）。
    DevicesUsb,
    Device {
        path: String,
    },
    DockerSocket,
}

impl Permission {
    /// 高权限在安装确认时单独醒目展示。
    pub fn is_high(&self) -> bool {
        matches!(
            self,
            Self::NetworkLan
                | Self::EgressProxy
                | Self::NetworkHost
                | Self::Capability { .. }
                | Self::HostPid
                | Self::HostIpc
                | Self::SecurityOpt
                | Self::Privileged
                | Self::DevicesGpu
                | Self::DevicesSerial
                | Self::DevicesUsb
                | Self::Device { .. }
                | Self::DockerSocket
        )
    }
}

fn parse_access(value: &str) -> Option<Access> {
    match value {
        "read" => Some(Access::Read),
        "write" => Some(Access::Write),
        _ => None,
    }
}

impl FromStr for Permission {
    type Err = ManifestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let unknown = || ManifestError::UnknownPermission(value.to_owned());
        let simple = match value {
            "models" => Some(Self::Models),
            "tools" => Some(Self::Tools),
            "plugins.health" => Some(Self::PluginsHealth),
            "matrix.bot" => Some(Self::MatrixBot),
            "notify" => Some(Self::Notify),
            "owner.identity" => Some(Self::OwnerIdentity),
            "network.egress" => Some(Self::NetworkEgress),
            "egress.proxy" => Some(Self::EgressProxy),
            "network.lan" => Some(Self::NetworkLan),
            "network.clients" => Some(Self::NetworkClients),
            "network.host" => Some(Self::NetworkHost),
            "host.pid" => Some(Self::HostPid),
            "host.ipc" => Some(Self::HostIpc),
            "security.opt" => Some(Self::SecurityOpt),
            "privileged" => Some(Self::Privileged),
            "devices.gpu" => Some(Self::DevicesGpu),
            "devices.serial" => Some(Self::DevicesSerial),
            "devices.usb" => Some(Self::DevicesUsb),
            "docker.socket" => Some(Self::DockerSocket),
            _ => None,
        };
        if let Some(permission) = simple {
            return Ok(permission);
        }
        if let Some(name) = value.strip_prefix("cap.") {
            let name = name.to_ascii_uppercase();
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            {
                return Ok(Self::Capability { name });
            }
            return Err(unknown());
        }
        if let Some(rest) = value.strip_prefix("storage.share.") {
            let (name, access) = rest.split_once(':').ok_or_else(unknown)?;
            let access = parse_access(access).ok_or_else(unknown)?;
            if !is_label(name) {
                return Err(unknown());
            }
            return Ok(Self::StorageShare {
                name: name.to_owned(),
                access,
            });
        }
        if let Some(access) = value.strip_prefix("storage.home:") {
            let access = parse_access(access).ok_or_else(unknown)?;
            return Ok(Self::StorageHome { access });
        }
        if let Some(path) = value.strip_prefix("devices.")
            && path.starts_with('/')
        {
            return Ok(Self::Device {
                path: path.to_owned(),
            });
        }
        Err(unknown())
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let access = |access: &Access| match access {
            Access::Read => "read",
            Access::Write => "write",
        };
        match self {
            Self::StorageShare { name, access: a } => {
                write!(f, "storage.share.{name}:{}", access(a))
            }
            Self::StorageHome { access: a } => write!(f, "storage.home:{}", access(a)),
            Self::Models => f.write_str("models"),
            Self::Tools => f.write_str("tools"),
            Self::PluginsHealth => f.write_str("plugins.health"),
            Self::MatrixBot => f.write_str("matrix.bot"),
            Self::Notify => f.write_str("notify"),
            Self::OwnerIdentity => f.write_str("owner.identity"),
            Self::NetworkEgress => f.write_str("network.egress"),
            Self::EgressProxy => f.write_str("egress.proxy"),
            Self::NetworkLan => f.write_str("network.lan"),
            Self::NetworkClients => f.write_str("network.clients"),
            Self::Capability { name } => write!(f, "cap.{name}"),
            Self::HostPid => f.write_str("host.pid"),
            Self::HostIpc => f.write_str("host.ipc"),
            Self::SecurityOpt => f.write_str("security.opt"),
            Self::Privileged => f.write_str("privileged"),
            Self::DevicesGpu => f.write_str("devices.gpu"),
            Self::DevicesSerial => f.write_str("devices.serial"),
            Self::DevicesUsb => f.write_str("devices.usb"),
            Self::Device { path } => write!(f, "devices.{path}"),
            Self::DockerSocket => f.write_str("docker.socket"),
            Self::NetworkHost => f.write_str("network.host"),
        }
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
