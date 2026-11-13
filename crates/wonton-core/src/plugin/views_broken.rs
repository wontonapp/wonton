//! 安装记录不可读时，只展示列表里已知的信息。

use wonton_protocol::{InstalledPlugin, PluginDetail};

/// 记录读不出来的插件：只有列表里的那些信息。
pub(super) fn broken_detail(installed: InstalledPlugin) -> PluginDetail {
    // 记录读不出来，选过的设备也无从知道。

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
        scope: None,
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
        problems: Vec::new(),
        capability_providers: Vec::new(),
        lan_webui: None,
        save_grants: Vec::new(),
        ask_every_update: None,
        limits: None,
        shares: None,
    }
}
