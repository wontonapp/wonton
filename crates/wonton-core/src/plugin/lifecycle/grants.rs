//! 安装时交给部署链路的主机探测结果：显卡、局域网端口和设备。

use super::super::gpu::GpuGrant;
use super::super::lan::LanGrant;

/// 安装时按主机的探测结果算出来的东西：显卡、局域网端口和设备（ADR-0018、ADR-0019、ADR-0036）。
pub(in crate::plugin) struct Grants {
    pub gpu: Option<GpuGrant>,
    pub lan: Option<LanGrant>,
    pub devices: super::super::device::DeviceGrant,
    pub device_record: Vec<super::super::store::InstalledDevice>,
}
