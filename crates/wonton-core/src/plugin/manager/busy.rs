//! “正在进行”的回答和独立任务的执行壳（见 [`super::manager`]）。

use std::future::Future;

use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginId, PluginStatus};

/// 插件正在安装、卸载或升级时，对它的新操作如实回答“正在进行”。
pub(in crate::plugin) fn busy(
    id: &PluginId,
    status: &PluginStatus,
    langs: &Languages,
) -> Option<InstallError> {
    let doing = match status {
        PluginStatus::Installing => "plugin-install-busy-installing",
        PluginStatus::Uninstalling => "plugin-install-busy-uninstalling",
        PluginStatus::Upgrading => "plugin-install-busy-upgrading",
        PluginStatus::Running | PluginStatus::Stopped | PluginStatus::Degraded { .. } => {
            return None;
        }
    };
    Some(InstallError::Conflict(l10n::text(
        langs,
        doing,
        &[("id", id.to_string().into())],
    )))
}

/// 在独立的任务里执行，调用方只等结果：调用方被丢弃（请求断开：手机切网、隧道断开）时操作照样做完，不会停在
/// 半路，留下一直“安装中”“卸载中”的插件和装了一半的资源。任务不会被取消（只有运行时关闭时才会，那时调用方
/// 也已经没了），所以出错只可能是任务 panic，原样传给调用方。
pub(in crate::plugin) async fn detached<T: Send + 'static>(
    op: impl Future<Output = T> + Send + 'static,
) -> T {
    match tokio::spawn(op).await {
        Ok(out) => out,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}
