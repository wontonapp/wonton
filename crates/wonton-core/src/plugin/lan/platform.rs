//! 桌面虚拟机：服务器跑在电脑上的虚拟机里（Docker Desktop、OrbStack、WSL2），虚拟机自己的地址手机连不到。
//!
//! 只按内核串和 DMI 认，不按网卡类型或网段猜：Proxmox、ESXi、Unraid 里桥接的虚拟机是普通发行版内核，WSL 的随机 172.x 也可能
//! 撞上真实的家庭网段。Lima 的内核也是普通发行版，认不出，由启动器声明宣告地址。

use super::probe::HostNetwork;

/// 从内核串和 DMI 里认出来的桌面虚拟机：跑在电脑上，虚拟机自己的地址手机连不到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopVm {
    /// 给主人看的名字。
    pub product: &'static str,
    /// 认出来的原文：内核串和 DMI，写进说明的详情。
    pub evidence: String,
}

/// 内核串里的标记，只按这几个词认，不按网卡类型或网段猜。
const KERNELS: [(&str, &str); 3] = [
    ("linuxkit", "Docker Desktop"),
    ("orbstack", "OrbStack"),
    ("microsoft-standard-wsl2", "WSL2"),
];

/// DMI 的型号和厂商里的标记。Docker Desktop 和 OrbStack 的 DMI 字符串没在真机核实过，只作内核串的补充；WSL2 的 DMI 是
/// 通用 Hyper-V 的值，按它认会把桥接的 Hyper-V 虚拟机也当成连不到，所以只认内核串。
const DMI: [(&str, &str); 2] = [("docker", "Docker Desktop"), ("orbstack", "OrbStack")];

/// 内核串或 DMI 里有没有认得出的桌面虚拟机标记。
pub fn desktop_vm(network: &HostNetwork) -> Option<DesktopVm> {
    let kernel = network.kernel.as_deref().unwrap_or_default().to_lowercase();
    let dmi = network.dmi.as_deref().unwrap_or_default().to_lowercase();
    let product = KERNELS
        .iter()
        .find(|(marker, _)| kernel.contains(marker))
        .or_else(|| DMI.iter().find(|(marker, _)| dmi.contains(marker)))
        .map(|(_, product)| *product)?;
    let evidence: Vec<&str> = [network.kernel.as_deref(), network.dmi.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    Some(DesktopVm {
        product,
        evidence: evidence.join(" / "),
    })
}
