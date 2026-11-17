//! 清单的 `devices`：按类声明要交给哪些服务（ADR-0036）。宿主上的具体设备由主人安装时选。

pub use wonton_protocol::manifest::{DeviceClass, DeviceSlot, ManifestDevices};
