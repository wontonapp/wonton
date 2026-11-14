//! 管理接口给 App 的视图（ADR-0017 的“接口：App 和核心”）：已安装的插件、详情页、安装预览和图标。文字按请求的
//! 语言挑好，App 原样显示。

<<<<<<< HEAD
mod detail;
mod install;
mod list;
mod text;
=======
use std::sync::Arc;

use wonton_protocol::{
    CardLine, EgressProblem, InstallPreview, InstalledPlugin, InstalledPlugins, PluginAuthor,
    PluginChat, PluginDetail, PluginHealth, PluginKind as Kind, PluginLimits, PluginState,
    PluginTab, PluginTrust, PluginWriter, RequirementLine, RequirementStatus,
};

use super::egress::probe::blocks;
use super::host_check::{self, Item};
use super::host_facts::HostFacts;
use super::image_worker::ImageError;
use super::lan::webui;
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{Entry, Health, InstallError, PluginManager, PluginStatus, supported};
use super::manifest::{Manifest, Permission, PluginId, PluginKind, Sso};
use super::manifest_tabs;
use super::package::PluginPackage;
use super::render::{Deployment, dry_run};
use super::views_broken::broken_detail;
use super::{
    gallery::IconSource,
    github_source::{Origin, Trust},
};
use super::{
    store::{EnabledPermission, InstalledRecord},
    views_source::source_notices,
};
use crate::l10n;

impl PluginManager {
    /// 挡安装的最后两条：代理不通，或者套接字路径放不下（数据目录、中继目录被指到了很长的路径）。
    fn blocking_problem(
        &self,
        m: &Manifest,
        plan: &Deployment,
        langs: &Languages,
        egress: Option<&EgressProblem>,
    ) -> Option<String> {
        egress
            .filter(|p| blocks(p))
            .map(|p| p.message.clone())
            .or_else(|| {
                super::host_relay::path_too_long(
                    &self.cfg.relay_dir,
                    &self.cfg.instance,
                    &m.id.slug(),
                    &plan.relays,
                    langs,
                )
            })
    }

    /// 宿主网络的服务怎么出网：配了代理又声明了 `network.egress` 时经代理，否则走宿主自己的路由（ADR-0036）。
    fn host_egress_notice(&self, m: &Manifest, langs: &Languages) -> Option<CardLine> {
        if !m.permissions.contains(&Permission::NetworkHost) {
            return None;
        }
        let proxied = m.permissions.contains(&Permission::NetworkEgress)
            && self
                .egress_config()
                .is_ok_and(|config| config.url.is_some());
        let key = if proxied {
            "install-host-network-proxied"
        } else {
            "install-host-network-egress"
        };
        Some(line(l10n::text(langs, key, &[])))
    }

    /// 已安装的插件：系统插件在前，其余按安装的先后。
    pub fn installed(&self, langs: &Languages) -> InstalledPlugins {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut listed: Vec<_> = plugins
            .iter()
            .map(|(id, entry)| {
                let installed_at = entry.listed.as_ref().map_or(0, |l| l.installed_at);
                (
                    (!entry.system, installed_at),
                    self.listing(&plugins, id, entry, langs),
                )
            })
            .chain(
                self.failed_installs
                    .read()
                    .expect("安装结果锁中毒")
                    .iter()
                    .map(|(id, failed)| {
                        (
                            (true, failed.entry.checked_at),
                            self.failed_listing(&plugins, id, failed, langs),
                        )
                    }),
            )
            .collect();
        drop(plugins);
        listed.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.id.cmp(&y.id)));
        InstalledPlugins {
            plugins: listed
                .into_iter()
                .map(|(_, plugin)| self.with_dependency_status(self.with_bot(plugin), langs))
                .collect(),
            upgrade_check: self.upgrades.problem(),
        }
    }

    /// 一个已安装的插件；没装时为空。
    pub fn installed_plugin(&self, id: &PluginId, langs: &Languages) -> Option<InstalledPlugin> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let plugin = match plugins.get(id) {
            Some(entry) => self.listing(&plugins, id, entry, langs),
            None => {
                let failed = self.failed_installs.read().expect("安装结果锁中毒");
                return Some(self.failed_listing(&plugins, id, failed.get(id)?, langs));
            }
        };
        drop(plugins);
        Some(self.with_dependency_status(self.with_bot(plugin), langs))
    }

    /// 给主人的详情，返回详情的接口都走这里：要求逐条判断过（要问 Docker），开关开着时带上第三方客户端的连接信息。
    pub async fn judged_detail(&self, id: &PluginId, langs: &Languages) -> Option<PluginDetail> {
        let mut detail = self.installed_detail(id, langs)?;
        detail.lan_webui = self.with_lan_client(id, detail.lan_webui.take()).await;
        let listed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id).and_then(|entry| entry.listed.clone())
        };
        if let Some(listed) = listed {
            detail.requirements =
                requirement_checks(&listed.manifest, &self.host_facts(langs).await, langs);
        }
        Some(detail)
    }

    /// 已安装插件的详情页（装着的版本），不带要求；没装时为空。上传的插件包没有来源，也就没有截图。
    pub fn installed_detail(&self, id: &PluginId, langs: &Languages) -> Option<PluginDetail> {
        let listed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id).and_then(|entry| entry.listed.clone())
        };
        let installed = self.installed_plugin(id, langs)?;
        let Some(listed) = listed else {
            return Some(broken_detail(installed));
        };
        let needs_configuration = installed.status == PluginState::NeedsConfiguration;
        let m = &listed.manifest;
        let origin = listed.origin.as_ref();
        let record = self.store.load(id).ok().flatten();
        // “每次都问”只有 Agent 写的插件才有（ADR-0033）。
        let ask_every_update = installed.written_by.is_some().then(|| {
            record
                .as_ref()
                .is_some_and(|record| record.ask_every_update)
        });
        Some(PluginDetail {
            source: origin.map(|o| o.repo.clone()),
            provider: origin.map(Origin::provider),
            via_index: origin.and_then(Origin::via_index),
            id: installed.id.clone(),
            title: installed.title.clone(),
            summary: installed.summary.clone(),
            about: m.about.as_ref().map(|t| langs.pick_owned(t)),
            icon: installed.icon.clone(),
            screenshots: Vec::new(),
            tags: langs.pick_tags(&m.tags),
            kind: installed.kind,
            runtime: m.runtime.into(),
            security_tier: Some(super::security_tier::security_tier(m)),
            link_url: m.link.as_ref().map(|link| link.url.clone()),
            scope: Some(wonton_protocol::PluginScope::All),
            audience: Some(super::audience::view(m)),
            installable: true,
            install_gate: wonton_protocol::Gate::Allow,
            version: m.version.clone(),
            trust: installed.trust,
            author: author(m),
            homepage: m.homepage.clone(),
            repository: origin.map(Origin::repository_url),
            license: m.license.clone(),
            license_url: m.license_url.clone(),
            stars: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            permissions: super::views_permissions::permission_lines(
                m,
                record
                    .as_ref()
                    .map_or(&[][..], |record| &record.enabled_permissions),
                langs,
            ),
            widget_capabilities: super::manifest_widget::capabilities(m),
            requirements: Vec::new(),
            secret_inputs: self.installed_input_states(id, m, langs),
            has_web: m.webui.is_some(),
            has_settings: super::settings::enabled(m),
            installed: Some(installed),
            blocked: None,
            notices: setup_notice(m, langs)
                .into_iter()
                .chain(
                    listed
                        .firewalled
                        .then(|| line(l10n::text(langs, "plugin-firewall-notice", &[])))
                        .map(|mut notice| {
                            notice.emphasis = true;
                            notice
                        }),
                )
                .chain(cross_origin_isolation_notice(m, langs))
                .chain(super::manifest_widget::manifest_notices(m, langs))
                .chain(self.unfilled_notice(needs_configuration, id, m, langs))
                .chain(
                    super::egress::provider::provides_proxy(m)
                        .then(|| line(l10n::text(langs, "plugin-egress-proxy-select", &[]))),
                )
                .collect(),
            hints: self
                .models_hint(id, m)
                .into_iter()
                .chain(self.dependency_hints(m, langs))
                .chain(self.hosted_hint(m))
                .collect(),
            problems: record
                .as_ref()
                .map(|record| super::device::problems(record, self.host.dev.as_deref(), langs))
                .unwrap_or_default(),
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            capability_providers: self.consumer_capabilities(m, langs).unwrap_or_default(),
            lan_webui: listed
                .lan_switched
                .as_ref()
                .map(|switched| switched.offer(listed.lan_webui, langs)),
            save_grants: self.save_grants(id),
            // 只有 Agent 写的插件才有这个开关的值（ADR-0033）。
            ask_every_update,
            limits: Some(listed.limits),
            shares: None,
        })
    }

    /// 安装插件包之前给主人确认的内容（按来源取来的插件包带着来源）。插件包不合法时报错；合法但这台主机上装不了时，原因写在 `blocked`。
    pub async fn preview_install(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let blocked = blocked_by(self.precheck(&package.manifest, langs).await)?;
        let mut preview = self
            .preview_with(package, langs, blocked, None, &[], None)
            .await?;
        if let Some(notice) = self.retained_data(&package.manifest.id, langs).await? {
            let after_review_notice = usize::from(preview.trust != PluginTrust::Official);
            preview.notices.insert(after_review_notice, notice);
        }
        Ok(preview)
    }

    /// 没装着、却留着这个 ID 的数据（不删数据的卸载留下的）时的提示：新装的会接着用它们，不管它来自哪里（ADR-0031）。
    pub(crate) async fn retained_data(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<Option<CardLine>, InstallError> {
        let volumes = !self.own_resources(id).await?.volumes.is_empty();
        let secrets = self.store.secrets().load(id)?.is_some();
        let dir = self.cfg.apps_dir.join(id.slug()).exists() || self.storage_retained(id);
        Ok((volumes || secrets || dir).then(|| CardLine {
            text: l10n::text(
                langs,
                "install-retained-data",
                &[("id", id.to_string().into())],
            ),
            emphasis: true,
        }))
    }

    /// 要主人确认的内容，`blocked` 是已经知道的装不了的原因。安装和升级共用。
    pub(super) async fn preview_with(
        &self,
        package: &PluginPackage,
        langs: &Languages,
        blocked: Option<String>,
        written_by: Option<PluginWriter>,
        enabled: &[EnabledPermission],
        installed: Option<&InstalledRecord>,
    ) -> Result<InstallPreview, InstallError> {
        super::install_entry::check_runtime(&package.manifest, langs)?;
        let settings = settings_form(package, installed, langs)?;
        let plan = dry_run(package, langs)?;
        let icon = self.check_icon(package, langs).await?;
        if let Some(icon) = &icon {
            self.gallery.remember_preview(icon);
        }
        let m = &package.manifest;
        // 显卡：本机会给什么、或者为什么给不了；required 而给不了也是这台主机上装不了的原因。
        let granted = m.effective_permissions(&super::store::permission_set(enabled));
        let gpu = self
            .gpu_preview(m, granted.contains(&Permission::DevicesGpu), langs)
            .await?;
        let lan = self.lan_preview(package, langs).await?;
        let egress = self.egress_problem_for(m, langs).await?;
        let dependencies = self.dependency_preview(m, langs).await?;
        let devices = self.devices_preview(m, enabled, installed, langs).await?;
        let mut blocked = blocked
            .or(dependencies.blocked)
            .or(gpu.blocked)
            .or(lan.blocked)
            .or_else(|| devices.blocked.as_ref().map(|(message, _)| message.clone()));
        if blocked.is_none() {
            blocked = self.blocking_problem(m, &plan, langs, egress.as_ref());
        }
        let blocked_category = devices.blocked.as_ref().and_then(|(message, category)| {
            (blocked.as_ref() == Some(message)).then_some(*category)
        });
        let mut notices = self.preview_notices(package, &plan, langs).await;
        notices.extend(
            gpu.notice
                .into_iter()
                .chain(lan.notices)
                .chain(devices.notices),
        );
        // 不挡安装的问题：探测只试了一个网站。
        let uncertain = egress.filter(|p| !blocks(p));
        notices.extend(uncertain.map(|p| line(p.message)));
        notices.extend(dependencies.notices);
        // 缺默认模型的提醒不挡安装：先接模型，Bot 才能回话。只读模型列表，不发计费补全。
        if let Some(notice) = self.model_route_notice(m, langs).await {
            notices.push(notice);
        }
        Ok(InstallPreview {
            source: package.origin.as_ref().map(Origin::source),
            provider: package.origin.as_ref().map(Origin::provider),
            via_index: package.origin.as_ref().and_then(Origin::via_index),
            written_by,
            commit: package.origin.as_ref().map(|o| o.commit.clone()),
            digest: package.digest(),
            plan: self.dependency_plan(package)?,
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            version: m.version.clone(),
            kind: kind(m.kind),
            has_bot: super::bots::is_bot(m),
            runtime: m.runtime.into(),
            security_tier: Some(super::security_tier::security_tier(m)),
            scope_choices: self.scope_choices()?,
            icon: icon.map(|i| i.paths()),
            trust: trust(package.origin.as_ref()),
            author: author(m),
            license: m.license.clone(),
            license_url: m.license_url.clone(),
            permissions: super::views_permissions::permission_lines(m, enabled, langs),
            widget_capabilities: super::manifest_widget::capabilities(m),
            secret_inputs: super::secret_input::lines(m, &|_| false, false, langs),
            settings,
            notices,
            audience: Some(super::audience::view(m)),
            install_gate: wonton_protocol::Gate::Allow,
            missing_requires: dependencies.missing_requires,
            missing_wants: dependencies.missing_wants,
            wanting_plugins: dependencies.wanting_plugins,
            upgrade_from: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            blocked,
            blocked_category,
            // 协议及客户端保留此字段，以便新客户端继续处理旧核心的资料门禁。
            missing_profile: None,
            devices: Some(devices.items).filter(|items| !items.is_empty()),
            lan_webui: webui::switched(package).map(|switched| switched.offer(false, langs)),
            always_allowed: Vec::new(),
        })
    }

    async fn preview_notices(
        &self,
        package: &PluginPackage,
        plan: &super::render::Deployment,
        langs: &Languages,
    ) -> Vec<CardLine> {
        let m = &package.manifest;
        let mut notices = source_notices(package, plan, langs);
        notices.splice(0..0, opening_notices(m, langs));
        notices.extend(requirement_notices(m, &self.host_facts(langs).await, langs));
        notices.extend(self.host_egress_notice(m, langs));
        notices.extend(super::manifest_widget::manifest_notices(m, langs));
        notices
    }

    /// 处理插件包的图标：不合规格时安装失败（插件规范的“图标和截图”）。没有图标时为空。
    pub(super) async fn check_icon(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<Option<IconSource>, InstallError> {
        let icon = IconSource::of(package);
        if let Some(icon) = &icon {
            self.gallery
                .render(icon)
                .await
                .map_err(|e| Self::image_error(&icon.path, e, langs))?;
        }
        for (index, tab) in package
            .manifest
            .tabs
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            for (field, raw) in tab.icon_refs(index) {
                let source = IconSource::of_path(package, raw).ok_or_else(|| {
                    InstallError::Invalid(l10n::text(
                        langs,
                        "plugin-install-install-icon-file-missing",
                        &[("field", field.as_str().into()), ("raw", raw.into())],
                    ))
                })?;
                self.gallery
                    .render_tab(&source)
                    .await
                    .map_err(|e| Self::image_error(&format!("{field} {raw}"), e, langs))?;
            }
        }
        Ok(icon)
    }

    fn image_error(path: &str, error: ImageError, langs: &Languages) -> InstallError {
        match error {
            ImageError::Refused(reason) | ImageError::TimedOut(reason) => {
                InstallError::Invalid(l10n::text(
                    langs,
                    "plugin-install-install-icon-error",
                    &[("path", path.into()), ("reason", reason.into())],
                ))
            }
            ImageError::Unavailable(reason) => InstallError::ImagesUnavailable(reason),
        }
    }

    /// 这台主机上装不了的原因，和安装时的检查一致，但什么也不改。
    async fn precheck(&self, m: &Manifest, langs: &Languages) -> Result<(), InstallError> {
        if m.kind == PluginKind::System {
            return Err(InstallError::Unsupported(l10n::text(
                langs,
                "install-system-preinstalled",
                &[],
            )));
        }
        {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            if let Some(entry) = plugins.get(&m.id) {
                return Err(InstallError::Conflict(l10n::text(
                    langs,
                    "install-already-installed",
                    &[
                        ("id", m.id.to_string().into()),
                        ("version", entry.version.as_str().into()),
                    ],
                )));
            }
            let slug = m.id.slug();
            if let Some(other) = plugins.keys().find(|id| id.slug() == slug) {
                return Err(InstallError::Conflict(format!(
                    "和已安装的 {other} 用同一个网页地址"
                )));
            }
        }
        self.fits_host(m, langs).await
    }

    /// 核心支持它要的权限，这台主机放得下它。
    pub(crate) async fn fits_host(
        &self,
        m: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        super::install_entry::check_runtime(m, langs)?;
        if let Some(p) = m.permissions.iter().find(|p| !supported(p)) {
            return Err(InstallError::Unsupported(l10n::text(
                langs,
                "plugin-install-permission-unsupported",
                &[("permission", p.to_string().into())],
            )));
        }
        self.check_requirements(m, langs).await
    }

    /// 升级预览里这台主机上升不了的原因。
    pub(super) async fn host_block(
        &self,
        m: &Manifest,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        if m.kind == PluginKind::System {
            return Ok(Some(l10n::text(langs, "install-system-preinstalled", &[])));
        }
        blocked_by(self.fits_host(m, langs).await)
    }

    /// 详情页顶上那条“还没填哪些密钥”：和状态一样只在运行中给，别的状态有自己的提示。
    fn unfilled_notice(
        &self,
        needs_configuration: bool,
        id: &PluginId,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Option<CardLine> {
        if !needs_configuration {
            return None;
        }
        let missing = self.unfilled_secrets(id, Some(manifest));
        (!missing.is_empty()).then(|| {
            let names: Vec<&str> = missing.iter().map(String::as_str).collect();
            let mut line = line(l10n::text(
                langs,
                "plugin-secret-required-missing",
                &[("names", l10n::list(langs, &names).into())],
            ));
            line.emphasis = true;
            line
        })
    }

    /// 列表和详情里的状态：运行中、但必填的用户填写密钥还没填的是“待配置”（ADR-0064）。
    /// 只覆盖运行中：异常、停下、正在安装时那几种原因更要紧。
    fn plugin_state(
        &self,
        id: &PluginId,
        status: PluginState,
        manifest: Option<&Manifest>,
    ) -> PluginState {
        if matches!(status, PluginState::Running { .. })
            && !self.unfilled_secrets(id, manifest).is_empty()
        {
            return PluginState::NeedsConfiguration;
        }
        status
    }

    fn with_dependency_status(
        &self,
        mut plugin: InstalledPlugin,
        langs: &Languages,
    ) -> InstalledPlugin {
        if !matches!(plugin.status, PluginState::Running { .. }) {
            return plugin;
        }
        let listed = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .iter()
            .find(|(id, _)| id.as_str() == plugin.id)
            .and_then(|(_, entry)| entry.listed.clone());
        if let Some(listed) = listed
            && let Some(reason) = self.dependency_problem(&listed.manifest, langs)
        {
            plugin.status = PluginState::Degraded { reason };
        }
        plugin
    }

    /// 列表里的一项，还没填 Bot。
    pub(super) fn listing(
        &self,
        plugins: &std::collections::BTreeMap<PluginId, Entry>,
        id: &PluginId,
        entry: &Entry,
        langs: &Languages,
    ) -> InstalledPlugin {
        let manifest = entry.listed.as_ref().map(|l| &l.manifest);
        let origin = entry.listed.as_ref().and_then(|l| l.origin.as_ref());
        let (upgrade, mut notices) =
            self.upgrades
                .listing(id, &entry.version, &entry.status, langs);
        let tabs = entry
            .listed
            .as_ref()
            .and_then(|listed| listed.manifest.tabs.as_ref().map(|tabs| (listed, tabs)))
            .map(|(listed, tabs)| {
                tabs.iter()
                    .zip(&listed.tab_icons)
                    .filter_map(|(tab, (icon, selected))| {
                        Some(PluginTab {
                            id: tab.id.clone(),
                            title: langs.pick_owned(&tab.title),
                            icon: icon.as_ref()?.tab_image(),
                            icon_selected: selected.as_ref().map(IconSource::tab_image),
                            path: tab.path.clone(),
                            layout: manifest_tabs::layout(&tab.layout)?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if entry.status == PluginStatus::Running {
            notices.extend(self.oom_notices(id, langs));
        }
        InstalledPlugin {
            chat: manifest
                .filter(|m| super::bots::is_bot(m))
                .map(|m| PluginChat {
                    form: m.bot.as_ref().map_or_default(|bot| bot.chat_layout),
                }),
            id: id.to_string(),
            title: langs.pick_owned(&entry.title),
            summary: manifest.map_or_else(String::new, |m| langs.pick_owned(&m.description)),
            version: entry.version.clone(),
            revision: entry
                .listed
                .as_ref()
                .map_or(&entry.version, |listed| &listed.revision)
                .clone(),
            kind: manifest.map_or(Kind::Other, |m| kind(m.kind)),
            runtime: manifest.map_or(wonton_protocol::PluginRuntime::Other, |m| m.runtime.into()),
            icon: entry
                .listed
                .as_ref()
                .and_then(|l| l.icon.as_ref())
                .map(IconSource::paths),
            trust: if entry.system {
                PluginTrust::System
            } else {
                trust(origin)
            },
            source: origin.map(Origin::source),
            provider: origin.map(Origin::provider),
            via_index: origin.and_then(Origin::via_index),
            written_by: None,
            status: self.plugin_state(
                id,
                state(
                    &super::install_hosted::effective_status(plugins, entry, langs),
                    entry.health,
                ),
                manifest,
            ),
            web_url: self.open_url(id, entry),
            link_url: manifest
                .and_then(|m| m.link.as_ref())
                .map(|link| link.url.clone()),
            cross_origin_isolated: manifest
                .and_then(|m| m.webui.as_ref())
                .is_some_and(|webui| webui.cross_origin_isolated),
            declares_tools: manifest.is_some_and(|m| m.mcp.is_some()),
            tabs,
            bot_user_id: None,
            upgrade,
            removable: !entry.system,
            notices,
            enabled_root_permissions: Vec::new(),
            install_progress: entry.install_progress.clone(),
            install_failure: None,
            tunnel_ports: manifest.and_then(|m| m.tunnel.as_ref()).map_or_else(
                Vec::new,
                |tunnel| {
                    tunnel
                        .ports
                        .iter()
                        .map(|spec| wonton_protocol::MappedPort {
                            port: spec.port,
                            protocol: spec.protocol,
                        })
                        .collect()
                },
            ),
            matrix_widget: manifest.and_then(super::manifest_widget::plugin_widget),
        }
    }

    pub(super) fn set_listed_limits(&self, id: &PluginId, limits: PluginLimits) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id)
            && let Some(listed) = entry.listed.clone()
        {
            entry.listed = Some(Arc::new(Listed {
                limits,
                ..(*listed).clone()
            }));
        }
    }
}

/// 检查的结果：这台主机上装不了时是原因，别的错误原样报。
fn blocked_by(checked: Result<(), InstallError>) -> Result<Option<String>, InstallError> {
    match checked {
        Ok(()) => Ok(None),
        Err(InstallError::Unsupported(reason) | InstallError::Conflict(reason)) => Ok(Some(reason)),
        Err(e @ InstallError::GpuUnavailable(_)) => Ok(Some(e.to_string())),
        Err(e) => Err(e),
    }
}
>>>>>>> 3cb6f4f2 (refactor(plugin): 按职责拆分设备安装准备与损坏详情视图，提取设备预览夹具)

pub(crate) use super::views_agent_logins::agent_login_options;
pub(crate) use super::views_permissions::permission_line;
pub use super::views_permissions::permission_text;

<<<<<<< HEAD
pub(in crate::plugin) use detail::requirement_checks;
pub(crate) use text::cross_origin_isolation_notice;
pub(in crate::plugin) use text::{author, kind, line};
=======
pub(super) fn requirement_checks(
    manifest: &Manifest,
    facts: &HostFacts,
    langs: &Languages,
) -> Vec<RequirementLine> {
    manifest
        .requirements
        .as_ref()
        .map(|r| host_check::judge(r, facts, langs))
        .unwrap_or_default()
        .into_iter()
        .map(|check| check.line)
        .collect()
}

/// 安装预览里的要求。显卡不满足时不写：预览里关于显卡的那行（[`super::gpu::GpuPreview`]）已经写了为什么。
fn requirement_notices(manifest: &Manifest, facts: &HostFacts, langs: &Languages) -> Vec<CardLine> {
    let Some(requirements) = &manifest.requirements else {
        return Vec::new();
    };
    host_check::judge(requirements, facts, langs)
        .into_iter()
        .filter(|check| {
            let bad = matches!(
                check.line.status,
                RequirementStatus::Unmet | RequirementStatus::Degraded
            );
            !(check.item == Item::Gpu && bad)
        })
        .map(|check| match check.line.problem {
            Some(problem) => CardLine {
                text: format!("{}：{problem}", check.line.text),
                emphasis: true,
            },
            None => CardLine {
                text: check.line.text,
                emphasis: false,
            },
        })
        .collect()
}

/// 不是系统插件时：按来源装的看来源，上传的是本地插件。
pub(super) fn trust(origin: Option<&Origin>) -> PluginTrust {
    match origin.map(|o| o.trust) {
        Some(Trust::Official) => PluginTrust::Official,
        Some(Trust::Community) => PluginTrust::Community,
        None => PluginTrust::Local,
    }
}

pub(crate) fn cross_origin_isolation_notice(
    manifest: &Manifest,
    langs: &Languages,
) -> Option<CardLine> {
    if !manifest
        .webui
        .as_ref()
        .is_some_and(|webui| webui.cross_origin_isolated)
    {
        return None;
    }
    Some(CardLine {
        text: l10n::text(langs, "plugin-cross-origin-isolation", &[]),
        emphasis: true,
    })
}

/// 清单里的 `setup`：主人装之前就得看到的一步，醒目显示。
fn setup_notice(m: &super::manifest::Manifest, langs: &Languages) -> Option<CardLine> {
    m.setup.as_ref().map(|setup| CardLine {
        text: langs.pick_owned(setup),
        emphasis: true,
    })
}

/// 装好之后第一眼的说明：作者写的“装好后要做的一步”，以及用自己登录的网页、会露出身份的插件。
fn opening_notices(m: &Manifest, langs: &Languages) -> Vec<CardLine> {
    let mut notices = Vec::new();
    if let Some(setup) = setup_notice(m, langs) {
        notices.push(setup);
    }
    if m.webui.as_ref().is_some_and(|w| w.sso != Sso::None) {
        notices.push(line(l10n::text(langs, "install-webui-sso", &[])));
    }
    // 写了权限的，已经在权限列表里了。
    if super::views_permissions::reveals_owner_identity(m)
        && !m.permissions.contains(&Permission::OwnerIdentity)
    {
        notices.push(line(permission_text(&Permission::OwnerIdentity, langs)));
    }
    notices
}

pub(super) fn line(text: String) -> CardLine {
    CardLine {
        text,
        emphasis: false,
    }
}

pub(super) fn author(manifest: &Manifest) -> Option<PluginAuthor> {
    manifest.author.as_ref().map(|a| PluginAuthor {
        name: a.name.clone(),
        url: a.url.clone(),
    })
}

/// 设置 Schema 先校验：不合格在动资源之前就报。升级时把装着的值填进新 Schema 的表单。
fn settings_form(
    package: &PluginPackage,
    installed: Option<&InstalledRecord>,
    langs: &Languages,
) -> Result<Option<wonton_protocol::SettingsForm>, InstallError> {
    let Some(schema) = super::settings::load(package, langs)? else {
        return Ok(None);
    };
    let mut values = installed
        .map(|record| record.settings.clone())
        .unwrap_or_default();
    super::settings::value::defaults(&schema, &mut values);
    Ok(Some(super::settings::install_form(&schema, &values, langs)))
}

fn state(status: &PluginStatus, health: Option<Health>) -> PluginState {
    match status {
        PluginStatus::Installing => PluginState::Installing,
        PluginStatus::Stopped => PluginState::Stopped,
        PluginStatus::Running => PluginState::Running {
            health: match health {
                Some(Health::Healthy) => PluginHealth::Healthy,
                Some(Health::Unknown) | None => PluginHealth::Unknown,
            },
        },
        PluginStatus::Degraded { reason } => PluginState::Degraded {
            reason: reason.clone(),
        },
        PluginStatus::Uninstalling => PluginState::Uninstalling,
        PluginStatus::Upgrading => PluginState::Upgrading,
    }
}

pub(super) fn kind(kind: PluginKind) -> Kind {
    match kind {
        PluginKind::App => Kind::App,
        PluginKind::Agent => Kind::Agent,
        PluginKind::Channel => Kind::Channel,
        PluginKind::Model => Kind::Model,
        PluginKind::System => Kind::System,
    }
}
>>>>>>> 3cb6f4f2 (refactor(plugin): 按职责拆分设备安装准备与损坏详情视图，提取设备预览夹具)
