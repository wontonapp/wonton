//! 假数据入口里的安装预览和卸载预览：一个要 `x-acme.download-client` 的动画自动下载器，提供方是 qBittorrent。
//! 界面测试看确认页怎么列提供方、它的权限和密钥，以及卸载提供方时的拦截；没有后端，确认后不会真的装。
//!
//! 卸载被挡时列出依赖方，卸载使用方时不列（它没人再用）。

use wonton_protocol::{
    CapabilityProvider, CapabilityProviderRef, CoreError, InstallPreview, InstallProvider,
    PluginAction, PluginTrust, SecretInput, SelectedProvider, StoreConfirmation, UninstallOutcome,
};

use super::ClientCore;

pub(super) const PROVIDER_SOURCE: &str = "github:example/wonton-qbittorrent";
pub(super) const CONSUMER_SOURCE: &str = "github:example/wonton-autobangumi";

pub(super) fn provider_id() -> String {
    id_of(PROVIDER_SOURCE)
}

pub(super) fn consumer_id() -> String {
    id_of(CONSUMER_SOURCE)
}

/// 商店那一行用的 ID：不和装着的那一份撞（撞了商店详情会被当成已装的，只给“打开”）。
pub(super) fn store_id() -> String {
    format!("{}-store", id_of(CONSUMER_SOURCE))
}

pub(super) fn provider_store_id() -> String {
    format!("{}-store", id_of(PROVIDER_SOURCE))
}

/// 装着的那份的“旧核”替身：同样的提供方，但升级预览里 `upgrade_connects` 为假。
pub(super) fn old_core_id() -> String {
    format!("{}-old-core", id_of(PROVIDER_SOURCE))
}

/// 这个来源的安装预览；假数据里没有别的来源。
pub(super) fn preview(source: &str) -> Option<InstallPreview> {
    match source {
        CONSUMER_SOURCE => Some(consumer()),
        PROVIDER_SOURCE => Some(provider()),
        _ => None,
    }
}

/// 点“更新”时给的预览：提供方拿出一项想用它的应用还要接上的能力。旧核那份不带 `upgrade_connects`，
/// 升级页只列不勾。别的插件假数据里升不了。
pub(super) fn upgrade_preview(id: &str) -> Option<InstallPreview> {
    let old_core = id == old_core_id();
    if !old_core && id != provider_id() {
        return None;
    }
    let mut preview = provider();
    preview.id = id.to_owned();
    preview.version = "1.1.0".into();
    preview.upgrade_from = Some("1.0.0".into());
    preview.upgrade_connects = !old_core;
    Some(preview)
}

/// 假数据下的插件操作：从商店点“获取”时直接把预览放进确认页，关掉确认页时清空。
/// 别的操作照旧交给调用方。
pub(super) fn dispatch(core: &ClientCore, action: &PluginAction) -> Result<bool, CoreError> {
    match action {
        PluginAction::InstallNow { source } => {
            let preview = preview(source).ok_or_else(|| CoreError::NotReady {
                reason: format!("no fake preview for {source}"),
            })?;
            core.views().plugins.send_modify(|view| {
                view.confirmation = Some(StoreConfirmation::Install { preview });
            });
            Ok(true)
        }
        PluginAction::UpgradeNow { plugin_id } => {
            let preview = upgrade_preview(plugin_id).ok_or_else(|| CoreError::NotReady {
                reason: format!("no fake upgrade preview for {plugin_id}"),
            })?;
            core.views().plugins.send_modify(|view| {
                view.confirmation = Some(StoreConfirmation::Upgrade { preview });
            });
            Ok(true)
        }
        PluginAction::DismissStoreConfirmation => {
            core.views()
                .plugins
                .send_modify(|view| view.confirmation = None);
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(super) fn outcome(id: &str) -> UninstallOutcome {
    if id == provider_id() {
        UninstallOutcome::InUse {
            dependents: vec![consumer_ref()],
        }
    } else {
        UninstallOutcome::Ready {
            unused_providers: vec![reference()],
        }
    }
}

fn reference() -> CapabilityProviderRef {
    CapabilityProviderRef {
        id: provider_id(),
        title: "qBittorrent".into(),
        high_privilege: true,
    }
}

fn consumer_ref() -> CapabilityProviderRef {
    CapabilityProviderRef {
        id: consumer_id(),
        title: "Autobangumi".into(),
        high_privilege: false,
    }
}

fn candidate() -> CapabilityProvider {
    CapabilityProvider {
        id: provider_id(),
        title: "qBittorrent".into(),
        source: PROVIDER_SOURCE.into(),
        high_privilege: true,
    }
}

fn provider() -> InstallPreview {
    let mut preview = base(PROVIDER_SOURCE, "qBittorrent");
    preview.trust = PluginTrust::Community;
    preview.provider = Some("example.com/example/wonton-qbittorrent".into());
    preview.permissions = vec![
        wonton_protocol::PermissionLine {
            permission: "storage.share.downloads:write".into(),
            text: "Write to the downloads folder".into(),
            risk: None,
            notice: Some("Finished downloads land in the downloads folder".into()),
            supported: true,
            added: false,
            optional: false,
            enabled: false,
            off_loss: None,
            opened: None,
        },
        // 第三方能力看提供方的权限定高权限：有了它，想用 qBittorrent 的已装应用才列得出来。
        wonton_protocol::PermissionLine {
            permission: "network.lan".into(),
            text: "Reach the home network".into(),
            risk: Some(wonton_protocol::PermissionRisk::High),
            notice: Some("Can reach other devices on the home network".into()),
            supported: true,
            added: false,
            optional: false,
            enabled: false,
            off_loss: None,
            opened: None,
        },
    ];
    preview.secret_inputs = vec![SecretInput {
        name: "api_key".into(),
        title: "API key".into(),
        description: Some("Create one in qBittorrent under Options, Web UI".into()),
        url: None,
        required: true,
    }];
    preview.wanting_plugins = vec![CapabilityProviderRef {
        id: "app.example.audiobook".into(),
        title: "Audio Book".into(),
        high_privilege: true,
    }];
    preview.notices.push(wonton_protocol::CardLine {
        text: "Autobangumi will be able to use the downloader through Wonton".into(),
        emphasis: true,
    });
    preview
}

fn consumer() -> InstallPreview {
    let mut preview = base(CONSUMER_SOURCE, "Autobangumi");
    preview.trust = PluginTrust::Community;
    preview.summary = "Follows the season and grabs new episodes".into();
    preview.missing_requires = vec![wonton_protocol::MissingCapability {
        capability: "x-acme.download-client".into(),
        third_party: true,
        providers: vec![candidate()],
    }];
    preview.missing_wants = vec![wonton_protocol::MissingCapability {
        capability: "x-acme.indexer".into(),
        third_party: true,
        providers: Vec::new(),
    }];
    preview.install_providers = vec![InstallProvider {
        capability: "x-acme.download-client".into(),
        third_party: true,
        providers: vec![candidate()],
        selected: Some(SelectedProvider {
            provider: candidate(),
            preview: provider(),
            secrets: Default::default(),
        }),
    }];
    preview
}

fn base(source: &str, title: &str) -> InstallPreview {
    InstallPreview {
        source: Some(source.into()),
        commit: None,
        digest: format!("sha256:{}", "ab".repeat(32)),
        plan: "ui-fake-plan".into(),
        id: id_of(source),
        title: title.into(),
        subtitle: None,
        summary: String::new(),
        version: "1.0.0".into(),
        kind: wonton_protocol::PluginKind::App,
        runtime: Default::default(),
        security_tier: None,
        remote: None,
        audience: None,
        install_gate: wonton_protocol::Gate::Allow,
        scope_choices: Vec::new(),
        has_bot: false,
        icon: None,
        trust: PluginTrust::Official,
        provider: None,
        via_index: Some("example.com".into()),
        written_by: None,
        author: None,
        license: Some("GPL-3.0-only".into()),
        license_url: None,
        permissions: Vec::new(),
        widget_capabilities: Vec::new(),
        secret_inputs: Vec::new(),
        settings: None,
        notices: Vec::new(),
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        wanting_plugins: Vec::new(),
        connected_plugins: Vec::new(),
        upgrade_connects: false,
        upgrade_from: None,
        release_notes: None,
        blocked: None,
        blocked_category: None,
        missing_profile: None,
        devices: None,
        lan_webui: None,
        always_allowed: Vec::new(),
        install_providers: Vec::new(),
    }
}

fn id_of(source: &str) -> String {
    format!(
        "app.example.{}",
        source.rsplit('/').next().unwrap_or_default()
    )
}
