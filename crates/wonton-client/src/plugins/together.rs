//! 确认页的决定翻成对核心的请求：安装时一起装的提供方（ADR-0053），以及卸载时先卸依赖方、
//! 一起卸没人再用的提供方。
//!
//! 拆出 [`tasks`](super::tasks)保持文件在门禁内；操作入口在 [`tasks::run`](super::tasks::run)。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{
    CoreError, InstallPreview, InstallRequest, InstallTogether, PluginFailure, PluginOperation,
    PluginProblem, SelectedProvider, UninstallRequest,
};

use super::PluginsLink;
use super::api::{CoreApiError, INSTALL_TIMEOUT};
use super::tasks::Outcome;
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

impl PluginsLink {
    /// `UninstallPlugin`：标题、图标和分类从已安装列表里取。先卸 `dependents`（保留数据），再卸这个；
    /// `providers` 是同一次请求里一起卸的、没人再用的提供方（ADR-0053）。被在用的依赖挡住时把依赖方
    /// 留在任务上，壳据此给出“一起卸载”或“先停它们”。
    pub(crate) fn uninstall(
        self: &Arc<Self>,
        id: String,
        delete_data: bool,
        providers: Vec<String>,
        dependents: Vec<String>,
    ) -> Result<(), CoreError> {
        let (title, icon) = {
            let view = self.view.borrow();
            let plugin = view
                .installed
                .as_ref()
                .and_then(|installed| installed.plugins.iter().find(|plugin| plugin.id == id));
            plugin.map_or_else(
                || (id.clone(), None),
                |plugin| (plugin.title.clone(), plugin.icon.clone()),
            )
        };
        self.begin_task(&id, &title, icon, PluginOperation::Uninstall)?;
        let link = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let sent = async {
                for dependent in &dependents {
                    let deleted = api
                        .delete_detailed(
                            &["plugins", dependent.as_str()],
                            &[],
                            None::<&UninstallRequest>,
                            INSTALL_TIMEOUT,
                        )
                        .await;
                    if let Err(error) = deleted {
                        return Err(match error {
                            CoreApiError::Http {
                                message, plugins, ..
                            } if !plugins.is_empty() => PluginProblem {
                                reason: PluginFailure::InUse,
                                message: l10n!(
                                    CATALOG,
                                    language(),
                                    "plugins-tasks-dependent-in-use",
                                    id = dependent.as_str(),
                                    detail = message
                                ),
                                min_version: None,
                                settings: None,
                            },
                            error => error.into_problem(),
                        });
                    }
                }
                let query: &[(&str, &str)] = if delete_data {
                    &[("purge", "true")]
                } else {
                    &[]
                };
                let path = ["plugins", id.as_str()];
                let request = (!providers.is_empty()).then(|| UninstallRequest {
                    providers: providers.clone(),
                });
                let body = request.as_ref();
                match api
                    .delete_detailed(&path, query, body, INSTALL_TIMEOUT)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(CoreApiError::Http {
                        message, plugins, ..
                    }) if !plugins.is_empty() => {
                        link.set_task_dependents(&id, plugins);
                        Err(PluginProblem {
                            reason: PluginFailure::InUse,
                            message,
                            min_version: None,
                            settings: None,
                        })
                    }
                    Err(error) => Err(error.into_problem()),
                }
            };
            link.finish_task(&id, Outcome::Removed, sent).await;
        });
        Ok(())
    }

    /// 记下预览：失败后重试要来源，聊天列表的安装行要知道它是不是 Bot 插件。
    pub(in crate::plugins) fn remember_preview(&self, preview: &InstallPreview) {
        self.previews
            .lock()
            .expect("the previews lock never panics")
            .insert(preview.id.clone(), preview.clone());
    }
}

/// 主人在确认页上打开了开关（[`PluginAction::InstallPlugin`](wonton_protocol::PluginAction::InstallPlugin)）。
pub(super) fn lan_webui_chosen(preview: &InstallPreview) -> bool {
    preview
        .lan_webui
        .as_ref()
        .is_some_and(|offer| offer.enabled)
}

/// 确认页上看到的就是授予的：预览里列出的必需权限，可选项不在这里（ADR-0057）。预览说装不了时不发请求。
pub(super) fn confirmed(preview: &InstallPreview) -> Result<Vec<String>, CoreError> {
    if let Some(blocked) = preview
        .blocked
        .as_ref()
        .or(preview.missing_profile.as_ref())
    {
        return Err(CoreError::NotReady {
            reason: blocked.clone(),
        });
    }
    Ok(preview
        .permissions
        .iter()
        .filter(|line| !line.optional)
        .map(|line| line.permission.clone())
        .collect())
}

/// 主人在确认页上打开了的可选项。
pub(super) fn enabled_permissions(preview: &InstallPreview) -> Vec<String> {
    preview
        .permissions
        .iter()
        .filter(|line| line.optional && line.enabled)
        .map(|line| line.permission.clone())
        .collect()
}

/// 预览里选中的提供方转成一起装的请求（ADR-0053）：同一个提供方补几个缺的能力只装一次。
/// 缺的必需能力没有选中提供方时给出“商店里没有提供方”。
pub(super) fn install_together(
    preview: &InstallPreview,
) -> Result<Vec<InstallTogether>, CoreError> {
    if preview.install_providers.is_empty() {
        return Ok(Vec::new());
    }
    let mut together: Vec<InstallTogether> = Vec::new();
    for section in &preview.install_providers {
        let Some(selected) = section.selected.as_ref() else {
            return Err(CoreError::NotReady {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "plugins-tasks-preview-no-provider",
                    capability = section.capability.as_str()
                ),
            });
        };
        // 同一个提供方补几个能力只装一次；几节里填的密钥合到这一份决定上。
        if let Some(existing) = together
            .iter_mut()
            .find(|together| together.id == selected.provider.id)
        {
            existing.request.secrets.extend(selected.secrets.clone());
            continue;
        }
        together.push(provider_install(selected)?);
    }
    Ok(together)
}

/// 一个提供方的安装决定：它预览里列出的权限照授、主人填的密钥带上，内层不再带别的依赖（ADR-0053）。
/// 提供方的权限和醒目提示已列在确认页，最高级的也走过确认（ADR-0036），不再要求单独装。
fn provider_install(selected: &SelectedProvider) -> Result<InstallTogether, CoreError> {
    let mut preview = selected.preview.clone();
    if let Some(blocked) = preview.blocked.clone().or(preview.missing_profile.clone()) {
        return Err(CoreError::NotReady { reason: blocked });
    }
    let grant = confirmed(&preview)?;
    let enabled_permissions = enabled_permissions(&preview);
    let lan_webui = lan_webui_chosen(&preview);
    let devices = PluginsLink::chosen_devices(&preview);
    let audience = preview
        .audience
        .take()
        .and_then(crate::selected_plugin_audience);
    let Some(source) = preview.source.take() else {
        return Err(CoreError::InvalidInput {
            reason: l10n!(CATALOG, language(), "plugins-tasks-preview-no-source"),
        });
    };
    Ok(InstallTogether {
        id: selected.provider.id.clone(),
        request: InstallRequest {
            source,
            digest: preview.digest,
            plan: preview.plan,
            scope: None,
            audience,
            grant,
            enabled_permissions,
            secrets: selected.secrets.clone(),
            settings: Default::default(),
            devices,
            lan_webui,
            install_together: Vec::new(),
            connect_plugins: Vec::new(),
            remote_oauth: None,
        },
    })
}
