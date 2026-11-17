//! 假数据入口中的设备详情和安装失败详情，供无后端的界面测试使用。

use wonton_protocol::{
    Gate, InstalledPlugin, PluginDetail, PluginDetailRoute, PluginDeviceKind, PluginFailure,
    PluginGrantedDevice, PluginProblem, PluginRuntime, PluginState,
};

pub(super) const HUB_TITLE: &str = "Home Hub";
pub(super) const HUB_ID: &str = "app.example.hub";

/// 网格上的那一格：串口不在了，状态是异常。
pub(super) fn status() -> PluginState {
    PluginState::Degraded {
        reason: GONE.into(),
    }
}

const GONE: &str = "Sonoff Zigbee 3.0 is not connected. Plug it back in.";

/// 未提供测试数据的详情照常走没有后端的路径。
pub(super) fn detail(
    route: &PluginDetailRoute,
    installed: Option<InstalledPlugin>,
) -> Option<PluginDetail> {
    let PluginDetailRoute::Installed { id } = route else {
        return None;
    };
    if id == HUB_ID {
        return Some(hub(installed));
    }
    let installed = installed.filter(|plugin| plugin.install_failure.is_some())?;
    let mut detail = hub(None);
    detail.id = installed.id.clone();
    detail.title = installed.title.clone();
    detail.summary = installed.summary.clone();
    detail.source = installed.source.clone();
    detail.kind = installed.kind;
    detail.version = installed.version.clone();
    detail.trust = installed.trust;
    detail.problems.clear();
    detail.devices.clear();
    detail.installed = Some(installed);
    Some(detail)
}

fn hub(installed: Option<InstalledPlugin>) -> PluginDetail {
    PluginDetail {
        source: None,
        id: HUB_ID.into(),
        title: HUB_TITLE.into(),
        summary: "Zigbee, USB scanners and disk health in one place".into(),
        about: None,
        icon: None,
        screenshots: Vec::new(),
        tags: Vec::new(),
        kind: wonton_protocol::PluginKind::App,
        runtime: PluginRuntime::default(),
        security_tier: None,
        link_url: None,
        remote: None,
        scope: None,
        audience: None,
        installable: true,
        install_gate: Gate::Allow,
        version: "1.0.0".into(),
        trust: wonton_protocol::PluginTrust::Official,
        provider: None,
        via_index: None,
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
        installed,
        blocked: None,
        notices: Vec::new(),
        hints: Vec::new(),
        problems: vec![PluginProblem {
            reason: PluginFailure::DeviceUnavailable,
            message: GONE.into(),
            min_version: None,
            settings: None,
        }],
        devices: vec![
            PluginGrantedDevice {
                path: "/dev/serial/by-id/usb-ITead_Sonoff_Zigbee_3.0-if00-port0".into(),
                name: Some("Sonoff Zigbee 3.0".into()),
                kind: PluginDeviceKind::Serial,
                system: false,
                available: false,
            },
            PluginGrantedDevice {
                path: "/dev/bus/usb".into(),
                name: None,
                kind: PluginDeviceKind::Usb,
                system: false,
                available: true,
            },
            disk(
                "nvme-Samsung_SSD_990_PRO_1TB",
                "Samsung SSD 990 PRO 1TB",
                true,
            ),
            disk("ata-WDC_WD40EFRX-68N32N0", "WDC WD40EFRX 4TB", false),
            PluginGrantedDevice {
                path: "/dev/example-hardware".into(),
                name: None,
                kind: PluginDeviceKind::Other,
                system: false,
                available: true,
            },
        ],
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        capability_providers: Vec::new(),
        lan_webui: None,
        save_grants: Vec::new(),
        limits: None,
        ask_every_update: None,
        shares: None,
    }
}

fn disk(by_id: &str, name: &str, system: bool) -> PluginGrantedDevice {
    PluginGrantedDevice {
        path: format!("/dev/disk/by-id/{by_id}"),
        name: Some(name.into()),
        kind: PluginDeviceKind::Disk,
        system,
        available: true,
    }
}
