//! 一键安装、一键升级（ADR-0017 的“一键更新”）：核心取预览、判断要不要请主人拿主意，不用拿主意的直接开始。
//! 要拿主意的放确认页（`PluginsViewModel.confirmation`），“全部更新”排进队列一次一条。

use std::collections::VecDeque;
use std::sync::Arc;

use wonton_protocol::{
    CoreError, InstallPreview, InstalledPlugin, PluginProblem, PluginState, PluginTask,
    PluginTaskState, PluginsViewModel, SecretValues, StoreConfirmation, StoreItem, UpdatablePlugin,
};

use super::PluginsLink;
use super::api::SOURCE_TIMEOUT;
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

/// 这份预览要不要主人看过才能装或升级。规则和确认页一致（ADR-0017），只是决定从壳挪进了核心：
/// 装不了、缺资料、要用设备、有核心不支持的权限、缺 `requires` 要一起装、widget 能力变多；安装时高风险权限、
/// widget 能力或醒目提醒（内存、架构、自动登录这类说明不拦）；更新时权限有变，或这个插件有 Bot 开了
/// “始终允许”（ADR-0024）。
fn needs_confirmation(preview: &InstallPreview, upgrade: bool) -> bool {
    if preview.blocked.is_some()
        || preview.missing_profile.is_some()
        || preview.install_gate == wonton_protocol::Gate::AskAdmin
        || (!upgrade
            && preview
                .audience
                .as_ref()
                .is_some_and(|view| !view.choices.is_empty() || view.notice.is_some()))
        || !preview.always_allowed.is_empty()
        || preview
            .devices
            .as_ref()
            .is_some_and(|devices| !devices.is_empty())
        || !preview.missing_requires.is_empty()
        || preview
            .permissions
            .iter()
            .any(|line| !line.supported || line.added)
        || preview
            .widget_capabilities
            .iter()
            .any(|capability| capability.added)
    {
        return true;
    }
    if upgrade {
        return false;
    }
    preview.notices.iter().any(|notice| notice.emphasis)
        || preview.permissions.iter().any(|line| line.risk.is_some())
        || !preview.widget_capabilities.is_empty()
}

/// 有新版本可升级的应用：核心查到新版本的，加上商店里版本号和装着的不一样的；正在装、升、卸的不算。
pub(super) fn updatable(view: &PluginsViewModel) -> Vec<UpdatablePlugin> {
    let installed = view.installed.iter().flat_map(|list| &list.plugins);
    installed
        .filter(|plugin| plugin.install_failure.is_none() && !is_busy(plugin))
        .filter_map(|plugin| {
            if let Some(offer) = &plugin.upgrade {
                return Some(updatable_of(plugin, &offer.version));
            }
            let item = store_item(view, &plugin.id)?;
            // 升级检查还没跑到时，商店里版本号和装着的不一样的也算（和行尾“更新”按钮同一个口径）。
            (item.installed_version.as_deref() != Some(item.version.as_str()))
                .then(|| updatable_of(plugin, &item.version))
        })
        .collect()
}

fn updatable_of(plugin: &InstalledPlugin, version: &str) -> UpdatablePlugin {
    UpdatablePlugin {
        plugin_id: plugin.id.clone(),
        title: plugin.title.clone(),
        version: version.to_owned(),
        icon: plugin.icon.clone(),
    }
}

/// “我的应用”之外还有能加的应用：小标题旁的“全部”和最后一行都进“全部应用”。
/// 点了一行“我的应用”打开什么：和 Bot 的私聊在前，插件网页在后；两个都没有的点不开，不进侧边栏。
fn is_busy(plugin: &InstalledPlugin) -> bool {
    matches!(
        plugin.status,
        PluginState::Installing | PluginState::Upgrading | PluginState::Uninstalling
    )
}

fn store_items(view: &PluginsViewModel) -> impl Iterator<Item = &StoreItem> {
    let official = view.store.iter().flat_map(|store| &store.official.plugins);
    let added = view
        .store
        .iter()
        .flat_map(|store| &store.added)
        .flat_map(|source| &source.section.plugins);
    let community = view.store.iter().flat_map(|store| &store.community.plugins);
    official.chain(added).chain(community)
}

fn store_item<'a>(view: &'a PluginsViewModel, id: &str) -> Option<&'a StoreItem> {
    store_items(view).find(|item| item.id == id)
}

impl PluginsLink {
    /// `InstallNow`：商店列表或详情页点了“获取”。
    pub(crate) fn install_now(self: &Arc<Self>, source: String) -> Result<(), CoreError> {
        let view = self.view.borrow();
        let id = store_items(&view)
            .find(|item| item.source == source)
            .map(|item| item.id.clone());
        drop(view);
        if self.now_in_flight(id.as_deref().unwrap_or(&source)) {
            return Ok(());
        }
        self.begin_pending(id.as_deref().unwrap_or(&source));
        let link = self.clone();
        self.runtime.spawn(async move {
            let preview = link.preview_for_install(&source).await;
            link.settle_now(id, preview, false).await;
        });
        Ok(())
    }

    /// `UpgradeNow`：列表行尾或详情页点了“更新”。
    pub(crate) fn upgrade_now(self: &Arc<Self>, plugin_id: String) -> Result<(), CoreError> {
        if self.now_in_flight(&plugin_id) {
            return Ok(());
        }
        self.begin_pending(&plugin_id);
        let link = self.clone();
        self.runtime.spawn(async move {
            let preview = link.preview_for_upgrade(&plugin_id).await;
            link.settle_now(Some(plugin_id), preview, true).await;
        });
        Ok(())
    }

    /// `UpgradeAll`：逐个升级可更新的应用；要拿主意的排进确认页，一次一条。
    pub(crate) fn upgrade_all(self: &Arc<Self>) -> Result<(), CoreError> {
        let ids: Vec<String> = self
            .view
            .borrow()
            .updatable
            .iter()
            .map(|updatable| updatable.plugin_id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let link = self.clone();
        self.runtime.spawn(async move {
            for plugin_id in ids {
                if link.now_in_flight(&plugin_id) {
                    continue;
                }
                link.begin_pending(&plugin_id);
                let preview = link.preview_for_upgrade(&plugin_id).await;
                link.settle_now(Some(plugin_id), preview, true).await;
            }
        });
        Ok(())
    }

    /// `DismissStoreConfirmation`：关掉确认页，没装也没升；还有排着队的就放下一条进来。
    pub(crate) fn dismiss_confirmation(&self) {
        let next = self.next_confirmation();
        self.view.send_modify(|view| view.confirmation = next);
    }

    /// 主人在确认页上点安装或升级（和 [`super::tasks`] 的入口同一处）：确认页清掉，放下一条。
    pub(crate) fn resolve_confirmation(self: &Arc<Self>, preview: &InstallPreview) {
        let confirmed = self
            .view
            .borrow()
            .confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation_preview(confirmation).id == preview.id);
        if confirmed {
            let next = self.next_confirmation();
            self.view.send_modify(|view| view.confirmation = next);
        }
    }

    async fn preview_for_install(&self, source: &str) -> Result<InstallPreview, PluginProblem> {
        let api = self.api()?;
        api.post(
            &["plugins", "preview"],
            &wonton_protocol::InstallSource {
                source: source.to_owned(),
            },
            SOURCE_TIMEOUT,
        )
        .await
    }

    async fn preview_for_upgrade(&self, id: &str) -> Result<InstallPreview, PluginProblem> {
        let api = self.api()?;
        api.post(
            &["plugins", id, "upgrade", "preview"],
            &serde_json::json!({}),
            SOURCE_TIMEOUT,
        )
        .await
    }

    /// 预览取回来了：要么直接开始，要么请主人拿主意；取不到就是一次失败的任务，行尾回到“更新”或“获取”。
    async fn settle_now(
        self: &Arc<Self>,
        key: Option<String>,
        preview: Result<InstallPreview, PluginProblem>,
        upgrade: bool,
    ) {
        if let Some(key) = &key {
            self.finish_pending(key);
        }
        match preview {
            Ok(preview) if needs_confirmation(&preview, upgrade) => {
                let confirmation = if upgrade {
                    StoreConfirmation::Upgrade { preview }
                } else {
                    StoreConfirmation::Install { preview }
                };
                self.offer_confirmation(confirmation);
            }
            Ok(preview) => {
                let started = if upgrade {
                    self.upgrade(preview, SecretValues::new(), Default::default())
                } else {
                    self.install(preview, SecretValues::new(), Default::default())
                };
                if let Err(error) = started {
                    // 预览来自家里服务器，总有来源；还剩“同一个插件已有任务在跑”（连点），那条任务就是反馈。
                    if !matches!(error, CoreError::NotReady { .. }) {
                        tracing::warn!(%error, "{}", l10n!(CATALOG, language(), "plugin-now-start-failed", error = error.to_string()));
                    }
                }
            }
            Err(problem) => self.now_failed(key, problem, upgrade),
        }
    }

    fn offer_confirmation(&self, confirmation: StoreConfirmation) {
        if self.view.borrow().confirmation.is_some() {
            self.queued_confirmations()
                .lock()
                .expect("the confirmations lock never panics")
                .push_back(confirmation);
            return;
        }
        self.view
            .send_modify(|view| view.confirmation = Some(confirmation));
    }

    fn next_confirmation(&self) -> Option<StoreConfirmation> {
        self.queued_confirmations()
            .lock()
            .expect("the confirmations lock never panics")
            .pop_front()
    }

    /// 排队换服务器时作废：字段挂在作用域上，跟别的一起换。
    fn queued_confirmations(&self) -> &std::sync::Mutex<VecDeque<StoreConfirmation>> {
        &self.confirmations
    }

    /// 取预览失败：留一条失败的任务，行下面一行红字配“重试”，按钮回到“更新”或“获取”。
    fn now_failed(&self, key: Option<String>, problem: PluginProblem, upgrade: bool) {
        let id = key.unwrap_or_else(|| "store".to_owned());
        let (title, icon) = self.display_of(&id);
        self.view.send_modify(|view| {
            view.tasks
                .retain(|task| !(task.id == id && !task.state.in_progress()));
            view.tasks.push(PluginTask {
                title,
                icon,
                operation: if upgrade {
                    wonton_protocol::PluginOperation::Upgrade
                } else {
                    wonton_protocol::PluginOperation::Install
                },
                state: PluginTaskState::Failed {
                    reason: problem.reason,
                    message: problem.message,
                },
                install_progress: None,
                waiting_for_connection: false,
                added_tools: None,
                id,
            });
            Self::derive(view);
        });
    }

    /// 商店列表里认得出的插件用它的标题和图标，认不出的用来源。
    fn display_of(&self, id: &str) -> (String, Option<wonton_protocol::PluginIcon>) {
        let view = self.view.borrow();
        if let Some(updatable) = view.updatable.iter().find(|item| item.plugin_id == id) {
            return (updatable.title.clone(), updatable.icon.clone());
        }
        if let Some(item) = store_item(&view, id) {
            return (item.title.clone(), item.icon.clone());
        }
        (id.to_owned(), None)
    }

    fn now_in_flight(&self, key: &str) -> bool {
        let view = self.view.borrow();
        let confirmed = view
            .confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation_preview(confirmation).id == key);
        confirmed
            || view.pending.iter().any(|pending| pending == key)
            || view
                .tasks
                .iter()
                .any(|task| task.id == key && task.state.in_progress())
            || self
                .queued_confirmations()
                .lock()
                .expect("the confirmations lock never panics")
                .iter()
                .any(|confirmation| confirmation_preview(confirmation).id == key)
    }

    fn begin_pending(&self, key: &str) {
        self.view.send_modify(|view| {
            if !view.pending.iter().any(|pending| pending == key) {
                view.pending.push(key.to_owned());
            }
        });
    }

    fn finish_pending(&self, key: &str) {
        self.view
            .send_modify(|view| view.pending.retain(|pending| pending != key));
    }
}

fn confirmation_preview(confirmation: &StoreConfirmation) -> &InstallPreview {
    match confirmation {
        StoreConfirmation::Install { preview } | StoreConfirmation::Upgrade { preview } => preview,
    }
}

#[cfg(test)]
mod tests {
    use wonton_protocol::{
        CardLine, MissingCapability, PermissionLine, PermissionRisk, PluginDevice, PluginKind,
        WidgetCapability, WidgetCapabilityLevel,
    };

    use super::*;

    /// 预览里的判定规则是写进 ADR-0017 的约定，这里逐条钉住。
    #[test]
    fn one_tap_installs_without_a_page_when_there_is_nothing_to_decide() {
        let plain = preview();
        assert!(!needs_confirmation(&plain, false));
        assert!(!needs_confirmation(&plain, true));
    }

    #[test]
    fn one_tap_asks_when_the_owner_must_decide() {
        let ask = |mut preview: InstallPreview, upgrade: bool| {
            preview.digest = format!("{}-{}", preview.digest, upgrade);
            needs_confirmation(&preview, upgrade)
        };
        assert!(ask(
            preview_with(|p| p.blocked = Some("架构不符".into())),
            false
        ));
        assert!(ask(
            preview_with(|p| p.missing_profile = Some("缺邮箱".into())),
            false
        ));
        // 设备选择只有真的有设备可挑时才要主人看；空列表没有可挑的。
        assert!(ask(
            preview_with(|p| {
                p.devices = Some(vec![PluginDevice {
                    id: "usb-1".into(),
                    name: "USB 摄像头".into(),
                    target: "/dev/video0".into(),
                    service: "camera".into(),
                    optional: false,
                }])
            }),
            false
        ));
        // 缺 requires（要一起装的提供方）：主人得看到，不挑就装不上。
        assert!(ask(
            preview_with(|p| {
                p.missing_requires = vec![MissingCapability {
                    capability: "model".into(),
                    third_party: false,
                    providers: Vec::new(),
                }]
            }),
            false
        ));
        assert!(ask(preview_with(|p| p.permissions[0].added = true), true));
        assert!(ask(
            preview_with(|p| p.permissions[0].supported = false),
            false
        ));
        assert!(ask(
            preview_with(|p| p.permissions[0].risk = Some(PermissionRisk::High)),
            false
        ));
        assert!(!ask(
            preview_with(|p| p.permissions[0].risk = Some(PermissionRisk::High)),
            true
        ));
        assert!(ask(
            preview_with(|p| p.notices = vec![CardLine {
                text: "占内存".into(),
                emphasis: true
            }]),
            false
        ));
        assert!(!ask(
            preview_with(|p| p.notices = vec![CardLine {
                text: "要 2 GB 内存".into(),
                emphasis: false
            }]),
            false
        ));
    }

    #[test]
    fn a_widget_page_asks_on_install_and_when_capabilities_grow() {
        let capability = || WidgetCapability {
            capability: "org.matrix.msc2762.send.state_event:m.room.member".into(),
            level: WidgetCapabilityLevel::High,
            added: false,
        };
        // 新装：能把网页当 widget 打开就要主人看过能力，不分级别。
        assert!(needs_confirmation(
            &preview_with(|p| p.widget_capabilities = vec![capability()]),
            false
        ));
        // 更新：能力没变不用再问，变多了才问。
        assert!(!needs_confirmation(
            &preview_with(|p| p.widget_capabilities = vec![capability()]),
            true
        ));
        assert!(needs_confirmation(
            &preview_with(|p| p.widget_capabilities = vec![WidgetCapability {
                added: true,
                ..capability()
            }]),
            true
        ));
    }

    fn preview() -> InstallPreview {
        InstallPreview {
            source: Some("github:owner/app.photos@v1.0.0".into()),
            commit: None,
            digest: "sha256:abc".into(),
            plan: "opaque-install-plan".into(),
            id: "app.photos".into(),
            title: "Photos".into(),
            summary: String::new(),
            license: None,
            license_url: None,
            version: "1.0.0".into(),
            kind: PluginKind::App,
            runtime: Default::default(),
            security_tier: None,
            scope_choices: Vec::new(),
            has_bot: false,
            icon: None,
            trust: wonton_protocol::PluginTrust::Official,
            provider: None,
            via_index: None,
            written_by: None,
            author: None,
            widget_capabilities: Vec::new(),
            permissions: vec![PermissionLine {
                permission: "storage.share.photos:read".into(),
                text: "读共享文件夹".into(),
                risk: None,
                notice: None,
                supported: true,
                added: false,
                optional: false,
                enabled: false,
                off_loss: None,
                opened: None,
            }],
            secret_inputs: Vec::new(),
            settings: None,
            notices: Vec::new(),
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            wanting_plugins: Vec::new(),
            upgrade_from: None,
            release_notes: None,
            missing_profile: None,
            blocked: None,
            blocked_category: None,
            devices: None,
            lan_webui: None,
            always_allowed: Vec::new(),
            audience: None,
            install_gate: wonton_protocol::Gate::Allow,
        }
    }

    fn preview_with(change: impl FnOnce(&mut InstallPreview)) -> InstallPreview {
        let mut preview = preview();
        change(&mut preview);
        preview
    }
}
