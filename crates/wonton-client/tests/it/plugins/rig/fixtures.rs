//! 造给客户端的 ViewModel：已安装插件和商店条目。

use wonton_client::{
    InstallPreview, InstalledPlugin, PermissionLine, PluginHealth, PluginKind, PluginState,
    PluginTrust, StoreItem, StoreListing, StoreSection,
};

pub fn installed(id: &str, status: PluginState) -> InstalledPlugin {
    InstalledPlugin {
        bot_danger_can_toggle: false,
        bot_shared: false,
        bot_danger: false,
        chat: Default::default(),
        id: id.to_owned(),
        title: format!("标题 {id}"),
        subtitle: None,
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: "1.0.0".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status,
        web_url: None,
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        default_bot_visibility: None,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
        media_browse: None,
        matrix_widget: None,
    }
}

pub fn running(id: &str) -> InstalledPlugin {
    installed(
        id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
    )
}

pub fn preview(id: &str) -> InstallPreview {
    InstallPreview {
        source: Some(format!("github:owner/{id}@v1.0.0")),
        commit: None,
        digest: "sha256:abc".to_owned(),
        plan: "opaque-install-plan".to_owned(),
        id: id.to_owned(),
        title: format!("标题 {id}"),
        subtitle: None,
        license: Some("AGPL-3.0-only".into()),
        license_url: None,
        summary: String::new(),
        version: "1.0.0".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        security_tier: None,
        remote: None,
        scope_choices: Vec::new(),
        has_bot: false,
        icon: None,
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        written_by: None,
        author: None,
        widget_capabilities: Vec::new(),
        permissions: vec![PermissionLine {
            permission: "storage.share.photos:read".to_owned(),
            text: "读共享文件夹 photos".to_owned(),
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
        connected_plugins: Vec::new(),
        upgrade_from: None,
        release_notes: None,
        missing_profile: None,
        blocked: None,
        blocked_category: None,
        devices: None,
        lan_webui: None,
        always_allowed: Vec::new(),
        audience: None,
        install_gate: wonton_client::Gate::Allow,
        install_providers: Vec::new(),
    }
}

pub fn item(id: &str, title: &str, tags: &[&str]) -> StoreItem {
    StoreItem {
        source: format!("github:owner/{id}"),
        id: id.to_owned(),
        title: title.to_owned(),
        subtitle: None,
        summary: String::new(),
        icon: None,
        tags: tags.iter().map(ToString::to_string).collect(),
        kind: PluginKind::App,
        version: "1.0.0".to_owned(),
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        author: None,
        license: None,
        license_url: None,
        stars: None,
        installed_version: None,
        security_tier: None,
        high_permissions: false,
        blocked: None,
        installable: true,
        install_gate: wonton_client::Gate::Allow,
    }
}

pub fn listing() -> StoreListing {
    let section = |plugins| StoreSection {
        plugins,
        fetched_at_ms: None,
        problem: None,
        unreadable: Vec::new(),
    };
    StoreListing {
        official: section(vec![item("app.photos", "Photos", &["相册"])]),
        added: Vec::new(),
        community: section(vec![
            item("app.music", "Music", &["音乐"]),
            item("app.more-photos", "Gallery", &["PHOTOS"]),
        ]),
    }
}
