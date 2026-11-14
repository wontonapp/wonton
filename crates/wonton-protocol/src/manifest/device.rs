use serde::{Deserialize, Serialize};

use super::Permission;

/// 清单的 `devices`：按类声明要交给哪些服务，宿主上的具体设备由主人选（ADR-0036）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestDevices {
    /// 每项是一个串口位置：主人给每个位置选一个设备。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serial: Vec<DeviceSlot>,
    /// 拿到整类 USB 设备的服务，不用选。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usb: Vec<DeviceSlot>,
    /// 拿到主人勾选的整块硬盘的服务。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disk: Vec<DeviceSlot>,
}

/// `devices` 里的一项：交给哪个服务，串口还要写容器里的路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSlot {
    pub service: String,
    /// 容器里的路径，只有串口要写：同一台主机上的设备路径每台都不一样，作者只能定容器里的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// 只有串口用：主人安装时可以不选。USB 和硬盘给的是整类或整张勾选的盘，勾了就是必备。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

impl ManifestDevices {
    /// 空的（`devices:` 下面一个类都没写）。
    pub fn is_empty(&self) -> bool {
        self.serial.is_empty() && self.usb.is_empty() && self.disk.is_empty()
    }

    /// 按类列出（服务, 位置）。
    pub fn classes(&self) -> [(DeviceClass, &[DeviceSlot]); 3] {
        [
            (DeviceClass::Serial, &self.serial),
            (DeviceClass::Usb, &self.usb),
            (DeviceClass::Disk, &self.disk),
        ]
    }

    /// 某一类的位置。
    pub fn slots(&self, class: DeviceClass) -> &[DeviceSlot] {
        match class {
            DeviceClass::Serial => &self.serial,
            DeviceClass::Usb => &self.usb,
            DeviceClass::Disk => &self.disk,
        }
    }
}

/// 设备类，和清单里的权限一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    Serial,
    Usb,
    Disk,
}

impl DeviceClass {
    /// 引用检查里用的字段名。
    pub fn field(self) -> &'static str {
        match self {
            Self::Serial => "devices.serial.service",
            Self::Usb => "devices.usb.service",
            Self::Disk => "devices.disk.service",
        }
    }
    pub const ALL: [Self; 3] = [Self::Serial, Self::Usb, Self::Disk];

    pub fn permission(self) -> Permission {
        match self {
            Self::Serial => Permission::DevicesSerial,
            Self::Usb => Permission::DevicesUsb,
            Self::Disk => Permission::DevicesDisk,
        }
    }

    /// 规范里的名字，用于报错。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::Usb => "usb",
            Self::Disk => "disk",
        }
    }
}
