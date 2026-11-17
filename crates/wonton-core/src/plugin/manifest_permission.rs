//! 清单权限的高权限表；权限名字的解析和显示由协议 crate 定义。
pub use wonton_protocol::manifest::{Access, Permission};

pub trait PermissionExt {
    fn is_high(&self) -> bool;
}

impl PermissionExt for Permission {
    /// 高权限在安装确认时单独醒目展示。
    fn is_high(&self) -> bool {
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
                | Self::DevicesDisk
                | Self::Device { .. }
                | Self::DockerSocket
        )
    }
}
