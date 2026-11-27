//! 期望状态：这一轮要不要入口和连接器、按哪个名字服务。

use std::path::{Path, PathBuf};

use super::{EntryContainers, EntryError};
use crate::entry_settings::EntrySettings;
use crate::home_service::Registration;

/// 一轮对账要服务的东西：名字来自设置文件（有名字的装法）或 App 里配的自己的域名（“不要名字”的装法），
/// 连同渲染配置要用的家名服务和解析器。设置文件那一份带着家名服务，入口对账不该自己再读一遍设置。
pub(super) struct Target {
    pub(super) domain: String,
    pub(super) registration: Option<Registration>,
    /// 校验 frps 证书用的 CA（有家名服务时才用）。
    pub(super) relay_ca: Option<PathBuf>,
    pub(super) resolvers: Vec<String>,
}

impl Target {
    /// 有设置文件就按设置；没有就看域名设置文件里有没有后缀。
    /// 名字和证书都在才有东西可服务，证书还没签出来时先不建，等签发后的下一轮对账。
    pub(super) fn resolve(
        install_dir: &Path,
        settings: Option<&EntrySettings>,
    ) -> Result<Option<Self>, EntryError> {
        let found = match settings {
            Some(settings) => Some((
                settings.domain.clone(),
                settings.registration.clone(),
                settings.relay_ca.clone(),
                settings.acme_resolvers.clone(),
            )),
            None => crate::clients::domain::stored_suffix(install_dir)
                .map_err(|error| EntryError::Settings(format!("{error:#}")))?
                .map(|suffix| (suffix, None, None, Vec::new())),
        };
        let Some((domain, registration, relay_ca, resolvers)) = found else {
            return Ok(None);
        };
        Ok(install_dir
            .join("entry/certs/site.pem")
            .is_file()
            .then_some(Self {
                domain,
                registration,
                relay_ca,
                resolvers,
            }))
    }
}

impl EntryContainers {
    /// 有联邦就要入口容器；连接器随总开关。
    ///
    /// 入口容器不随总开关停：配对过的服务器经 8449 的联邦不受总开关影响（ADR-0090:52），关掉时只清空
    /// 公网前端的证书列表。连接器的两种做法（frpc / 路由器转发）按设置里的家名服务选。
    pub(super) fn wanted(target: &Target, public: bool) -> (bool, bool) {
        let federation = !crate::matrix::federation_config::is_private(&target.domain);
        (federation, public)
    }
}
