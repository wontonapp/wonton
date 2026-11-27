//! 假数据入口里的“安装失败”应用（`WONTON_UI_FAILED_INSTALL`）和它的重试。
//!
//! 环境变量为 `noretry` 时，失败记录不可重试、也没有来源：模拟上传的包装失败后服务器没留下原决定（比如重启过），
//! 详情页该给“从文件重装”。其余取值是从仓库装、核心保留了原决定，可以直接重试。
//! 重试照真实核心的 `retry_install`：同样的拒绝条件，接受时只放一个转着的安装任务；没有后端，任务一直转。

use wonton_protocol::{
    CoreError, InstallFailure, InstallStage, InstalledPlugin, PluginAction, PluginOperation,
    PluginState,
};

use super::ClientCore;
use crate::plugins::PluginsLink;

const ID: &str = "app.example.failed";
/// 核心拉不到镜像时写的类别（`InstallError::ImagePull`），提示按它显示原因。
const KIND: &str = "image_pull_failed";
const MESSAGE: &str = "The image could not be downloaded.";

/// 环境变量开着时的那一格；没开时为空。
pub(super) fn plugin() -> Option<InstalledPlugin> {
    let mode = std::env::var("WONTON_UI_FAILED_INSTALL").ok()?;
    Some(fixture(mode != "noretry"))
}

fn fixture(retryable: bool) -> InstalledPlugin {
    let mut failed = super::ui_fake::fake_plugin(
        "Failed Example",
        ID,
        PluginState::Degraded {
            reason: MESSAGE.into(),
        },
        None,
    );
    failed.source = retryable.then(|| "github:example/failed-app@v1.0.0".into());
    failed.install_failure = Some(InstallFailure {
        kind: KIND.into(),
        message: MESSAGE.into(),
        stage: InstallStage::PullingImages,
        failed_plugin_id: ID.into(),
        cleanup_errors: Vec::new(),
        retryable,
    });
    failed
}

impl ClientCore {
    /// 插件操作里只接重试失败的安装，别的照旧不落到作用域。
    pub(super) fn dispatch_ui_fake_plugins(&self, action: PluginAction) -> Result<(), CoreError> {
        if super::ui_fake_install::dispatch(self, &action)? {
            return Ok(());
        }
        let PluginAction::RetryPluginInstall { id } = action else {
            return Ok(());
        };
        let mut outcome = Ok(());
        self.views().plugins.send_if_modified(|view| {
            outcome = PluginsLink::retry_target(view, &id).and_then(|(title, icon)| {
                PluginsLink::push_task(view, &id, &title, icon, PluginOperation::Install)
            });
            outcome.is_ok()
        });
        outcome
    }
}

#[cfg(test)]
mod tests {
    use wonton_protocol::{Action, PluginTaskState};

    use super::*;

    /// 不靠环境变量（进程级，并行的测试会互相改）：建好假核心后把这一格塞进已安装列表。
    fn core_with(retryable: bool, name: &str) -> (std::sync::Arc<ClientCore>, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("wonton-ui-fake-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
        core.views().plugins.send_modify(|view| {
            let installed = view.installed.as_mut().unwrap();
            installed.plugins.retain(|plugin| plugin.id != ID);
            installed.plugins.push(fixture(retryable));
        });
        (core, dir)
    }

    fn retry(core: &ClientCore) -> Result<(), CoreError> {
        core.dispatch(Action::Plugins {
            action: PluginAction::RetryPluginInstall { id: ID.into() },
        })
    }

    #[test]
    fn retry_starts_an_install_task_and_refuses_a_second_one() {
        let (core, dir) = core_with(true, "retry");
        retry(&core).unwrap();
        let task = core
            .plugins()
            .tasks
            .into_iter()
            .find(|task| task.id == ID)
            .unwrap();
        assert_eq!(task.operation, PluginOperation::Install);
        assert_eq!(task.state, PluginTaskState::Running);
        assert!(matches!(retry(&core), Err(CoreError::NotReady { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retry_is_refused_when_the_failure_kept_no_decision() {
        let (core, dir) = core_with(false, "noretry");
        assert!(matches!(retry(&core), Err(CoreError::NotReady { .. })));
        assert!(core.plugins().tasks.iter().all(|task| task.id != ID));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
