//! 安装期限和能力提示：清单能声明的安装期限（`install_timeout_secs`）怎么和核心的配置合并，以及容器起不来时
//! 怎么点名核心默认去掉的能力。

use std::time::Duration;

use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, ManifestError};
use super::render::ContainerPlan;
use crate::l10n;

/// 插件能申请的安装期限的上限。再长就是插件起不来，报超时比让主人无限等好。
pub const MAX_INSTALL_TIMEOUT_SECS: u64 = 1800;

/// 声明得比下限还短的安装期限没有意义，比核心默认的还小。
const MIN_INSTALL_TIMEOUT_SECS: u64 = 60;

/// 清单声明的安装期限在范围内。
pub(super) fn validate_install_timeout(manifest: &Manifest) -> Result<(), ManifestError> {
    let Some(secs) = manifest.install_timeout_secs else {
        return Ok(());
    };
    if (MIN_INSTALL_TIMEOUT_SECS..=MAX_INSTALL_TIMEOUT_SECS).contains(&secs) {
        return Ok(());
    }
    Err(ManifestError::Invalid(format!(
        "install_timeout_secs 须在 {MIN_INSTALL_TIMEOUT_SECS} 到 {MAX_INSTALL_TIMEOUT_SECS} 之间"
    )))
}

impl PluginManager {
    /// 主人等着的操作（安装、启动、升级、重建）最多等多久：核心的安装期限和清单声明的期限取大的，清单的上限在
    /// 校验里。监视和开机恢复是串行的，仍用固定的安装期限（见 [`super::health_gate::Gate`]）。
    pub(super) fn install_limit(&self, manifest: &Manifest) -> Duration {
        match manifest.install_timeout_secs {
            Some(secs) => self.cfg.install_timeout.max(Duration::from_secs(secs)),
            None => self.cfg.install_timeout,
        }
    }
}

/// 容器失败时，如果核心默认去掉了它可能需要的能力（宿主网络去掉 `NET_RAW`），在错误里点名。
/// 上游镜像不会说自己要它，只会在起来时 FATAL，没有这句提示作者只能去猜。
pub(super) fn note_dropped_capabilities(
    error: super::health::HealthError,
    container: &ContainerPlan,
    langs: &Languages,
) -> InstallError {
    let error = InstallError::from(error);
    let host = container.body.host_config.as_ref();
    let dropped = host
        .and_then(|host| host.cap_drop.as_ref())
        .is_some_and(|drops| drops.iter().any(|cap| cap == "NET_RAW"));
    if !dropped {
        return error;
    }
    error.with_note(
        langs,
        &l10n::text(
            langs,
            "plugin-install-capability-dropped",
            &[
                ("service", container.service.as_str().into()),
                ("capability", "NET_RAW".into()),
                ("permission", "cap.NET_RAW".into()),
                ("field", "cap_add: [NET_RAW]".into()),
            ],
        ),
    )
}
