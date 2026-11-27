//! 局域网端口和代理假地址的选路开关，拆出 [client mod] 保持文件在门禁内。

use super::*;

#[uniffi::export]
impl ClientCore {
    /// 打开后选路优先真直连和中继，把代理 TUN 的假地址（198.18.0.0/15）放到最后，只在别无他路时走它。
    /// 存在本机、所有服务器共用，写不了时返回 `Storage`；改动立刻重建 endpoint 生效。
    pub fn set_fake_ip_exclusion(&self, enabled: bool) -> Result<(), CoreError> {
        self.scope().home.set_fake_ip_exclusion(enabled)?;
        self.fake_ip_exclusion.store(enabled, Ordering::SeqCst);
        Ok(())
    }
    /// 代理的假地址是否排除出选路，默认关。
    pub fn fake_ip_exclusion(&self) -> bool {
        self.fake_ip_exclusion.load(Ordering::SeqCst)
    }
    /// 局域网设置：插件的局域网端口开在哪些地址上、可以选的网卡，以及开了端口的插件。
    pub async fn lan_settings(&self) -> Result<LanSettings, CoreError> {
        Err(lan_unsupported())
    }
    /// 改选局域网端口开在哪里。家里的服务器按新地址重建开了端口的插件容器，做完后返回新的设置；App 断开了它也
    /// 照样做完。选择不合法（一项都没选、网卡或地址不在宿主上、不是私有地址、[`LanChoice::Unknown`]），或者地址由
    /// `WONTON_LAN_ADDRESSES` 指定时返回 `InvalidInput`；选的网卡上没有私有 IPv4 地址、家里的服务器检测不了网卡时
    /// 返回 `NotReady`。
    pub async fn set_lan_choice(&self, choice: LanChoice) -> Result<LanSettings, CoreError> {
        let _ = choice;
        Err(lan_unsupported())
    }
}

/// 局域网设置的接口先于实现合并（ADR-0019）：实现之前如实失败。
fn lan_unsupported() -> CoreError {
    CoreError::NotReady {
        reason: l10n!(CATALOG, language(), "home-lan-unsupported"),
    }
}
