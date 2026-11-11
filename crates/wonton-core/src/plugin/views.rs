//! 管理接口给 App 的视图（ADR-0017 的“接口：App 和核心”）：已安装的插件、详情页、安装预览和图标。文字按请求的
//! 语言挑好，App 原样显示。

use std::sync::Arc;

use wonton_protocol::{
    CardLine, InstallPreview, InstalledPlugin, InstalledPlugins, PluginAuthor, PluginChat,
    PluginDetail, PluginHealth, PluginKind as Kind, PluginLimits, PluginState, PluginTab,
    PluginTrust, PluginWriter, RequirementLine, RequirementStatus,
};

use super::egress::probe::blocks;
use super::host_check::{self, Item};
use super::host_facts::HostFacts;
use super::image_worker::ImageError;
use super::lan::webui;
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{Entry, Health, InstallError, PluginManager, PluginStatus, supported};
use super::manifest::{AgentSpec, Manifest, Permission, PluginId, PluginKind, Sso};
use super::manifest_tabs;
use super::package::PluginPackage;
use super::render::{dry_run, dry_run_with_email};
use super::{
    gallery::{IconSource, Variant},
    github_source::{Origin, Trust},
};
use super::{store::EnabledPermission, views_source::source_notices};
use crate::l10n;

impl PluginManager {
    /// 已安装的插件：系统插件在前，其余按安装的先后。
    pub fn installed(&self, langs: &Languages) -> InstalledPlugins {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut listed: Vec<_> = plugins
            .iter()
            .map(|(id, entry)| {
                let installed_at = entry.listed.as_ref().map_or(0, |l| l.installed_at);
                (
                    (!entry.system, installed_at),
                    self.listing(id, entry, langs),
                )
            })
            .collect();
        drop(plugins);
        listed.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.id.cmp(&y.id)));
        InstalledPlugins {
            plugins: listed.into_iter().map(|(_, p)| self.with_bot(p)).collect(),
            upgrade_check: self.upgrades.problem(),
        }
    }

    /// 一个已安装的插件；没装时为空。
    pub fn installed_plugin(&self, id: &PluginId, langs: &Languages) -> Option<InstalledPlugin> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let plugin = self.listing(id, plugins.get(id)?, langs);
        drop(plugins);
        Some(self.with_bot(plugin))
    }

    /// 给主人的详情，返回详情的接口都走这里：要求逐条判断过（要问 Docker），开关开着时带上第三方客户端的连接信息。
    pub async fn judged_detail(&self, id: &PluginId, langs: &Languages) -> Option<PluginDetail> {
        let mut detail = self.installed_detail(id, langs)?;
        detail.lan_webui = self.with_lan_client(id, detail.lan_webui.take()).await;
        let listed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id)?.listed.clone()
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
            plugins.get(id)?.listed.clone()
        };
        let installed = self.installed_plugin(id, langs)?;
        let Some(listed) = listed else {
            return Some(broken_detail(installed));
        };
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
            scope: Some(wonton_protocol::PluginScope::All),
            version: m.version.clone(),
            trust: installed.trust,
            author: author(m),
            homepage: m.homepage.clone(),
            repository: origin.map(Origin::repository_url),
            license: m.license.clone(),
            stars: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            permissions: super::views_permissions::permission_lines(
                m,
                record
                    .as_ref()
                    .map_or(&[][..], |record| &record.enabled_permissions),
                langs,
            ),
            requirements: Vec::new(),
            secret_inputs: self.installed_input_states(id, m, langs),
            has_web: m.webui.is_some(),
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
                .chain(
                    super::egress::provider::provides_proxy(m)
                        .then(|| line(l10n::text(langs, "plugin-egress-proxy-select", &[]))),
                )
                .collect(),
            hints: self.models_hint(id, m).into_iter().collect(),
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            capability_providers: Vec::new(),
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
        let blocked = blocked_by(self.precheck(&package.manifest, langs).await)?;
        let mut preview = self
            .preview_with(package, langs, blocked, None, &[])
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
        let dir = self.cfg.apps_dir.join(id.slug()).exists();
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
    ) -> Result<InstallPreview, InstallError> {
        super::install_entry::check_runtime(&package.manifest, langs)?;
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
        let egress_block = egress.as_ref().filter(|p| blocks(p));
        let blocked = blocked
            .or(gpu.blocked)
            .or(lan.blocked)
            .or_else(|| egress_block.map(|p| p.message.clone()))
            .or_else(|| {
                super::host_relay::path_too_long(
                    &self.cfg.apps_dir.join(m.id.slug()),
                    &plan.relays,
                    langs,
                )
            });
        let mut notices = source_notices(package, &plan, langs);
        // 作者写的“装好后要做的一步”排在最前面、醒目显示：主人得先看到它再决定装不装。
        if let Some(setup) = setup_notice(m, langs) {
            notices.insert(0, setup);
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
        notices.extend(requirement_notices(m, &self.host_facts(langs).await, langs));
        if m.permissions.contains(&Permission::NetworkHost) {
            notices.push(line(l10n::text(langs, "install-host-network-egress", &[])));
        }
        notices.extend(gpu.notice);
        notices.extend(lan.notices);
        // 不挡安装的问题：探测只试了一个网站。
        let uncertain = egress.filter(|p| !blocks(p));
        notices.extend(uncertain.map(|p| line(p.message)));
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
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            version: m.version.clone(),
            kind: kind(m.kind),
            has_bot: super::bots::is_bot(m),
            runtime: m.runtime.into(),
            scope_choices: self.scope_choices()?,
            icon: icon.map(|i| i.paths()),
            trust: trust(package.origin.as_ref()),
            author: author(m),
            permissions: super::views_permissions::permission_lines(m, enabled, langs),
            secret_inputs: super::secret_input::lines(m, &|_| false, false, langs),
            notices,
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            wanting_plugins: Vec::new(),
            upgrade_from: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            blocked,
            blocked_category: None,
            missing_profile: self.missing_profile(package, langs),
            devices: None,
            lan_webui: webui::switched(package).map(|switched| switched.offer(false, langs)),
            always_allowed: Vec::new(),
        })
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
            ImageError::Refused(reason) => InstallError::Invalid(l10n::text(
                langs,
                "plugin-install-install-icon-error",
                &[("path", path.into()), ("reason", reason.into())],
            )),
            ImageError::Unavailable(reason) => InstallError::ImagesUnavailable(reason),
        }
    }

    /// 图标地址对应的 PNG：只有已安装的插件和最近预览过的插件包的图标。不认识的地址为 `Ok(None)`。
    pub async fn icon_png(
        &self,
        sha256: &str,
        variant: Variant,
    ) -> Result<Option<Vec<u8>>, ImageError> {
        let tab = matches!(variant, Variant::TabSmall | Variant::TabLarge);
        let installed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .values()
                .filter_map(|entry| entry.listed.as_ref())
                .flat_map(|listed| {
                    listed
                        .tab_icons
                        .iter()
                        .flat_map(|(icon, selected)| {
                            [icon.as_ref(), selected.as_ref()]
                                .into_iter()
                                .flatten()
                                .map(|icon| (icon.clone(), true))
                        })
                        .chain(listed.icon.as_ref().map(|icon| (icon.clone(), false)))
                })
                .find(|(icon, is_tab)| icon.sha256 == sha256 && *is_tab == tab)
        };
        let Some((source, is_tab)) = installed.or_else(|| {
            (!tab)
                .then(|| self.gallery.preview(sha256))
                .flatten()
                .map(|source| (source, false))
        }) else {
            return Ok(None);
        };
        let icon = if is_tab {
            self.gallery.render_tab(&source).await?
        } else {
            self.gallery.render(&source).await?
        };
        Ok(Some(match variant {
            Variant::Small | Variant::TabSmall => icon.small.clone(),
            Variant::Large | Variant::TabLarge => icon.large.clone(),
        }))
    }

    /// 这台主机上装不了的原因，和安装时的检查一致，但什么也不改。
    /// 主人还没填邮箱、插件又非要不可时的说明。
    pub(crate) fn missing_profile(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Option<String> {
        let needs_email = super::manifest_oidc::needs_owner_email(&package.manifest)
            || dry_run_with_email(package, None, &Languages::default()).is_err();
        (self.cfg.profile.email().is_none() && needs_email)
            .then(|| crate::owner::email_needed(langs))
    }

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

    /// 列表里的一项，还没填 Bot。
    fn listing(&self, id: &PluginId, entry: &Entry, langs: &Languages) -> InstalledPlugin {
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
                            display: manifest_tabs::display(&tab.display)?,
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
            status: state(&entry.status, entry.health),
            web_url: self.open_url(id, entry),
            web_display: manifest
                .and_then(|m| m.webui.as_ref())
                .and_then(|webui| manifest_tabs::display(&webui.display))
                .unwrap_or_default(),
            cross_origin_isolated: manifest
                .and_then(|m| m.webui.as_ref())
                .is_some_and(|webui| webui.cross_origin_isolated),
            declares_tools: manifest.is_some_and(|m| m.mcp.is_some()),
            tabs,
            bot_user_id: None,
            family: false,
            upgrade,
            removable: !entry.system,
            notices,
            enabled_root_permissions: Vec::new(),
            install_progress: entry.install_progress.clone(),
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

impl PluginManager {
    /// 已安装 Agent 的账号登录选项，文字按请求语言挑选。
    pub fn agent_logins(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Vec<wonton_protocol::AgentLogin> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.agent.as_ref())
            .map(|agent| agent_login_options(agent, langs))
            .unwrap_or_default()
    }
}

pub(crate) fn agent_login_options(
    agent: &AgentSpec,
    langs: &Languages,
) -> Vec<wonton_protocol::AgentLogin> {
    agent
        .login
        .iter()
        .map(|login| wonton_protocol::AgentLogin {
            id: login.id.clone(),
            title: langs.pick_owned(&login.title),
        })
        .collect()
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

pub(crate) use super::views_permissions::permission_line;
pub use super::views_permissions::permission_text;

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

/// 记录读不出来的插件：只有列表里的那些信息。
fn broken_detail(installed: InstalledPlugin) -> PluginDetail {
    PluginDetail {
        source: None,
        provider: None,
        via_index: None,
        id: installed.id.clone(),
        title: installed.title.clone(),
        summary: installed.summary.clone(),
        about: None,
        icon: None,
        screenshots: Vec::new(),
        tags: Vec::new(),
        kind: installed.kind,
        runtime: installed.runtime,
        scope: None,
        version: installed.version.clone(),
        trust: installed.trust,
        author: None,
        homepage: None,
        repository: None,
        license: None,
        stars: None,
        release_notes: None,
        permissions: Vec::new(),
        requirements: Vec::new(),
        secret_inputs: Vec::new(),
        has_web: false,
        installed: Some(installed),
        blocked: None,
        notices: Vec::new(),
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        hints: Vec::new(),
        capability_providers: Vec::new(),
        lan_webui: None,
        save_grants: Vec::new(),
        ask_every_update: None,
        limits: None,
        shares: None,
    }
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
