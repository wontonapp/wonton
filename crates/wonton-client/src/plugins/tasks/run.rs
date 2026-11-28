//! 操作入口：安装、重试、填密钥、升级。各步的公共细节（确认、设备、一起装的提供方）在
//! [`super::confirm`] 和 [`super::together`]。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, InstallPreview, InstallRequest, PluginDetail, PluginIcon, PluginOperation,
    PluginTaskState, PluginsViewModel, SecretString, SecretUpdate, SecretValues, SettingValues,
    UpgradeRequest,
};

use super::PluginsLink;
use super::api::{INSTALL_TIMEOUT, to_core_error};
use super::confirm::problem;
use super::finish::Outcome;
use super::together::{confirmed, enabled_permissions, install_together, lan_webui_chosen};
use crate::l10n::{CATALOG, language};

impl PluginsLink {
    /// 安装失败回滚后，核心保留原始安装决定时直接重装，不重新请求来源或权限确认。
    pub(crate) fn retry_install(self: &Arc<Self>, id: String) -> Result<(), CoreError> {
        let (title, icon) = Self::retry_target(&self.view.borrow(), &id)?;
        self.begin_task(&id, &title, icon, PluginOperation::Install)?;
        let link = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let path = ["plugins", id.as_str(), "retry"];
            let body = serde_json::json!({});
            let sent = api.post_unit(&path, &body, INSTALL_TIMEOUT);
            link.finish_task(&id, Outcome::Running { version: None }, sent)
                .await;
        });
        Ok(())
    }

    /// 能不能按保留的决定重试：结果不确定时不重试（可能已经在装），核心没保留决定的也不行。
    /// 能重试时给出任务行的标题和图标。UI 假核心也用它，和真实核心拒绝的情形一致。
    pub(crate) fn retry_target(
        view: &PluginsViewModel,
        id: &str,
    ) -> Result<(String, Option<PluginIcon>), CoreError> {
        if view
            .tasks
            .iter()
            .any(|task| task.id == id && matches!(task.state, PluginTaskState::Uncertain { .. }))
        {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "plugins-tasks-retry-unavailable"),
            });
        }
        let plugin = view
            .installed
            .as_ref()
            .and_then(|installed| installed.plugins.iter().find(|plugin| plugin.id == id))
            .ok_or_else(|| CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "plugins-tasks-no-such-task", id = id),
            })?;
        if !plugin
            .install_failure
            .as_ref()
            .is_some_and(|failure| failure.retryable)
        {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "plugins-tasks-retry-unavailable"),
            });
        }
        Ok((plugin.title.clone(), plugin.icon.clone()))
    }

    /// `InstallPlugin`：按预览安装，授予预览里列出的全部权限。
    pub(crate) fn install(
        self: &Arc<Self>,
        mut preview: InstallPreview,
        secrets: SecretValues,
        settings: SettingValues,
    ) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        self.resolve_confirmation(&preview);
        self.remember_preview(&preview);
        let lan_webui = lan_webui_chosen(&preview);
        let enabled_permissions = enabled_permissions(&preview);
        let audience = preview
            .audience
            .take()
            .and_then(crate::selected_plugin_audience);
        let devices = Self::chosen_devices(&preview);
        let missing = preview.missing_requires.clone();
        // 预览里每个缺的必需能力都带着选中的提供方；一起装的请求按预览原样带上（ADR-0053）。
        let decisions = install_together(&preview);
        let Some(source) = preview.source else {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "plugins-tasks-preview-no-source"),
            });
        };
        self.begin_task(
            &preview.id,
            &preview.title,
            preview.icon.clone(),
            PluginOperation::Install,
        )?;
        let mut request = InstallRequest {
            source,
            digest: preview.digest,
            plan: preview.plan,
            scope: None,
            audience,
            grant,
            enabled_permissions,
            secrets,
            settings,
            devices,
            lan_webui,
            install_together: Vec::new(),
            // 主人勾选的高权限使用方（ADR-0053 第 3 条）；提供方内层一律为空。
            connect_plugins: preview.connected_plugins.clone(),
            remote_oauth: None,
        };
        let link = self.clone();
        let id = preview.id;
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let sent = async {
                // 新预览带着每个提供方的完整决定；老预览（聊天导入、旧核心）只有候选、没有决定，
                // 静默挑第一个会漏掉最高权限的确认行，所以拒绝并让主人先单独装提供方。
                request.install_together = match decisions {
                    Ok(together) if !together.is_empty() => together,
                    Ok(_) if !missing.is_empty() => {
                        return Err(problem(CoreError::NotReady {
                            reason: l10n!(
                                CATALOG,
                                language(),
                                "plugins-tasks-preview-old-core",
                                capability = missing
                                    .first()
                                    .map_or("", |missing| missing.capability.as_str())
                            ),
                        }));
                    }
                    Ok(_) => Vec::new(),
                    Err(error) => return Err(problem(error)),
                };

                api.post_unit(&["plugins"], &request, INSTALL_TIMEOUT).await
            };
            link.finish_task(&id, Outcome::Running { version: None }, sent)
                .await;
        });
        Ok(())
    }

    /// 填一项密钥或清空可选项（ADR-0064）：核心重建用到它的服务，做完才回应。
    pub(crate) async fn set_secret_now(
        self: &Arc<Self>,
        id: String,
        name: String,
        value: Option<SecretString>,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = SecretUpdate { value };
        let result = api
            .put(
                &["plugins", &id, "secrets", &name],
                &update,
                INSTALL_TIMEOUT,
            )
            .await
            .map(super::identity::detail)
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }

    /// `UpgradePlugin`：升到预览给的版本。
    pub(crate) fn upgrade(
        self: &Arc<Self>,
        preview: InstallPreview,
        secrets: SecretValues,
        settings: SettingValues,
    ) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        self.resolve_confirmation(&preview);
        self.remember_preview(&preview);
        let lan_webui = lan_webui_chosen(&preview);
        self.begin_task(
            &preview.id,
            &preview.title,
            preview.icon.clone(),
            PluginOperation::Upgrade,
        )?;
        let enabled_permissions = enabled_permissions(&preview);
        let request = UpgradeRequest {
            devices: Self::chosen_devices(&preview),
            source: preview.source,
            digest: preview.digest,
            grant,
            enabled_permissions,
            secrets,
            settings,
            lan_webui,
        };
        let link = self.clone();
        let id = preview.id;
        let version = preview.version;
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let path = ["plugins", id.as_str(), "upgrade"];
            let sent = async {
                let bytes = api.post_raw(&path, &request, INSTALL_TIMEOUT).await?;
                if link.is_current(generation) {
                    link.set_added_tools(&id, &bytes);
                }
                Ok(())
            };
            let outcome = Outcome::Running {
                version: Some(version.clone()),
            };
            link.finish_task(&id, outcome, sent).await;
        });
        Ok(())
    }
}
