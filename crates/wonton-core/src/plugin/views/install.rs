//! 安装和升级前给主人确认的预览：显示的内容和这台主机装不装得下的检查。

use wonton_protocol::{
    CardLine, EgressProblem, InstallPreview, PluginTrust, PluginWriter, RequirementStatus,
};

use super::super::egress::probe::blocks;
use super::super::gallery::IconSource;
use super::super::github_source::Origin;
use super::super::host_check::{self, Item};
use super::super::host_facts::HostFacts;
use super::super::image_worker::ImageError;
use super::super::lan::webui;
use super::super::languages::Languages;
use super::super::manager::{InstallError, PluginManager, supported};
use super::super::manifest::{Manifest, Permission, PluginId, PluginKind};
use super::super::package::PluginPackage;
use super::super::render::{Deployment, dry_run};
use super::super::store::{EnabledPermission, InstalledRecord};
use super::super::views_source::source_notices;
use super::text::{author, kind, line, opening_notices, trust};
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
                super::super::host_relay::path_too_long(
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
    pub(in crate::plugin) async fn preview_with(
        &self,
        package: &PluginPackage,
        langs: &Languages,
        blocked: Option<String>,
        written_by: Option<PluginWriter>,
        enabled: &[EnabledPermission],
        installed: Option<&InstalledRecord>,
    ) -> Result<InstallPreview, InstallError> {
        super::super::install_entry::check_runtime(&package.manifest, langs)?;
        let settings = settings_form(package, installed, langs)?;
        let plan = dry_run(package, langs)?;
        let icon = self.check_icon(package, langs).await?;
        if let Some(icon) = &icon {
            self.gallery.remember_preview(icon);
        }
        let m = &package.manifest;
        // 显卡：本机会给什么、或者为什么给不了；required 而给不了也是这台主机上装不了的原因。
        let granted = m.effective_permissions(&super::super::store::permission_set(enabled));
        let gpu = self
            .gpu_preview(m, granted.contains(&Permission::DevicesGpu), langs)
            .await?;
        let lan = self.lan_preview(package, langs).await?;
        let egress = self.egress_problem_for(m, langs).await?;
        let dependencies = self.dependency_preview(m, langs).await?;
        let mut blocked = blocked
            .or(dependencies.blocked)
            .or(gpu.blocked)
            .or(lan.blocked);
        if blocked.is_none() {
            blocked = self.blocking_problem(m, &plan, langs, egress.as_ref());
        }
        let mut notices = source_notices(package, &plan, langs);
        // 作者写的“装好后要做的一步”排在最前面、醒目显示：主人得先看到它再决定装不装。
        notices.splice(0..0, opening_notices(m, langs));
        notices.extend(requirement_notices(m, &self.host_facts(langs).await, langs));
        notices.extend(self.host_egress_notice(m, langs));
        notices.extend(super::super::manifest_widget::manifest_notices(m, langs));
        notices.extend(gpu.notice);
        notices.extend(lan.notices);
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
            has_bot: super::super::bots::is_bot(m),
            runtime: m.runtime.into(),
            security_tier: Some(super::super::security_tier::security_tier(m)),
            scope_choices: self.scope_choices()?,
            icon: icon.map(|i| i.paths()),
            trust: trust(package.origin.as_ref()),
            author: author(m),
            license: m.license.clone(),
            license_url: m.license_url.clone(),
            permissions: super::super::views_permissions::permission_lines(m, enabled, langs),
            widget_capabilities: super::super::manifest_widget::capabilities(m),
            secret_inputs: super::super::secret_input::lines(m, &|_| false, false, langs),
            settings,
            notices,
            audience: Some(super::super::audience::view(m)),
            install_gate: wonton_protocol::Gate::Allow,
            missing_requires: dependencies.missing_requires,
            missing_wants: dependencies.missing_wants,
            wanting_plugins: dependencies.wanting_plugins,
            upgrade_from: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            blocked,
            blocked_category: None,
            // 协议及客户端保留此字段，以便新客户端继续处理旧核心的资料门禁。
            missing_profile: None,
            devices: None,
            lan_webui: webui::switched(package).map(|switched| switched.offer(false, langs)),
            always_allowed: Vec::new(),
        })
    }

    /// 处理插件包的图标：不合规格时安装失败（插件规范的“图标和截图”）。没有图标时为空。
    pub(in crate::plugin) async fn check_icon(
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
        super::super::install_entry::check_runtime(m, langs)?;
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
    pub(in crate::plugin) async fn host_block(
        &self,
        m: &Manifest,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        if m.kind == PluginKind::System {
            return Ok(Some(l10n::text(langs, "install-system-preinstalled", &[])));
        }
        blocked_by(self.fits_host(m, langs).await)
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

/// 安装预览里的要求。显卡不满足时不写：预览里关于显卡的那行（[`super::super::gpu::GpuPreview`]）已经写了为什么。
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

/// 设置 Schema 先校验：不合格在动资源之前就报。升级时把装着的值填进新 Schema 的表单。
fn settings_form(
    package: &PluginPackage,
    installed: Option<&InstalledRecord>,
    langs: &Languages,
) -> Result<Option<wonton_protocol::SettingsForm>, InstallError> {
    let Some(schema) = super::super::settings::load(package, langs)? else {
        return Ok(None);
    };
    let mut values = installed
        .map(|record| record.settings.clone())
        .unwrap_or_default();
    super::super::settings::value::defaults(&schema, &mut values);
    Ok(Some(super::super::settings::install_form(
        &schema, &values, langs,
    )))
}
