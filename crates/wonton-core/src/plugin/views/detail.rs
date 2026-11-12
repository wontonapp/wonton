//! 已安装插件的详情页视图。

use wonton_protocol::{
    CardLine, InstalledPlugin, PluginDetail, PluginState, RemoteAuthorization, RemoteMcpDetail,
    RequirementLine,
};

use super::super::github_source::Origin;
use super::super::host_check;
use super::super::host_facts::HostFacts;
use super::super::languages::Languages;
use super::super::manager::PluginManager;
use super::super::manifest::{Manifest, PluginId};
use super::text::{author, cross_origin_isolation_notice, line, setup_notice};
use crate::l10n;

/// 远端 MCP 插件的详情：授权状态由连接器记，核心还没有问它的路径时，要授权的先如实写“要授权”。
fn remote_detail(m: &Manifest) -> Option<RemoteMcpDetail> {
    m.remote_mcp.as_ref().map(|remote| RemoteMcpDetail {
        host: remote.host().to_owned(),
        auth: remote.protocol_auth(),
        authorization: match remote.auth {
            super::super::manifest_remote::RemoteAuth::Oauth => {
                RemoteAuthorization::NeedsAuthorization
            }
            _ => RemoteAuthorization::NotNeeded,
        },
    })
}

impl PluginManager {
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
            security_tier: Some(super::super::security_tier::security_tier(m)),
            link_url: m.link.as_ref().map(|link| link.url.clone()),
            remote: remote_detail(m),
            scope: Some(wonton_protocol::PluginScope::All),
            audience: Some(super::super::audience::view(m)),
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
            permissions: super::super::views_permissions::permission_lines(
                m,
                record
                    .as_ref()
                    .map_or(&[][..], |record| &record.enabled_permissions),
                langs,
            ),
            widget_capabilities: super::super::manifest_widget::capabilities(m),
            requirements: Vec::new(),
            secret_inputs: self.installed_input_states(id, m, langs),
            has_web: m.webui.is_some(),
            has_settings: super::super::settings::enabled(m),
            // 只有 Agent 写的插件才有这个开关的值（ADR-0033）。
            ask_every_update: installed.written_by.is_some().then(|| {
                record
                    .as_ref()
                    .is_some_and(|record| record.ask_every_update)
            }),
            installed: Some(installed),
            blocked: None,
            notices: self.installed_notices(needs_configuration, id, m, langs, listed.firewalled),
            hints: self
                .models_hint(id, m)
                .into_iter()
                .chain(self.dependency_hints(m, langs))
                .chain(self.hosted_hint(m))
                .collect(),
            // 选中的设备不在了就是一条 device_unavailable，其他问题详情页不背。
            problems: record
                .as_ref()
                .map(|record| {
                    super::super::device::problems(record, self.host.dev.as_deref(), langs)
                })
                .unwrap_or_default(),
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            capability_providers: self.consumer_capabilities(m, langs).unwrap_or_default(),
            lan_webui: listed
                .lan_switched
                .as_ref()
                .map(|switched| switched.offer(listed.lan_webui, langs)),
            save_grants: self.save_grants(id),
            limits: Some(listed.limits),
            shares: None,
        })
    }

    /// 已装详情页的提示条：先放行前的“装好后要做的一步”，再放防火墙、跨域隔离、清单提示、没填的密钥和出网代理。
    fn installed_notices(
        &self,
        needs_configuration: bool,
        id: &PluginId,
        m: &Manifest,
        langs: &Languages,
        firewalled: bool,
    ) -> Vec<CardLine> {
        setup_notice(m, langs)
            .into_iter()
            .chain(
                firewalled
                    .then(|| line(l10n::text(langs, "plugin-firewall-notice", &[])))
                    .map(|mut notice| {
                        notice.emphasis = true;
                        notice
                    }),
            )
            .chain(cross_origin_isolation_notice(m, langs))
            .chain(super::super::manifest_widget::manifest_notices(m, langs))
            .chain(self.unfilled_notice(needs_configuration, id, m, langs))
            .chain(
                super::super::egress::provider::provides_proxy(m)
                    .then(|| line(l10n::text(langs, "plugin-egress-proxy-select", &[]))),
            )
            .collect()
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
}

pub(in crate::plugin) fn requirement_checks(
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

/// 记录读不出来的插件：只有列表里的那些信息。
fn broken_detail(installed: InstalledPlugin) -> PluginDetail {
    PluginDetail {
        source: installed.source.clone(),
        provider: installed.provider.clone(),
        via_index: installed.via_index.clone(),
        id: installed.id.clone(),
        title: installed.title.clone(),
        summary: installed.summary.clone(),
        about: None,
        icon: installed.icon.clone(),
        screenshots: Vec::new(),
        tags: Vec::new(),
        kind: installed.kind,
        runtime: installed.runtime,
        security_tier: None,
        link_url: None,
        remote: None,
        scope: None,
        audience: None,
        installable: false,
        install_gate: wonton_protocol::Gate::Deny,
        version: installed.version.clone(),
        trust: installed.trust,
        author: None,
        homepage: None,
        repository: None,
        license: None,
        license_url: None,
        stars: None,
        release_notes: None,
        permissions: Vec::new(),
        widget_capabilities: Vec::new(),
        requirements: Vec::new(),
        secret_inputs: Vec::new(),
        has_web: false,
        has_settings: false,
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
        problems: Vec::new(),
    }
}
