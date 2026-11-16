//! 假数据入口（`ui_fake.rs`）里一个拿了设备的已装应用的详情页（ADR-0036）：一个串口不在了，USB 整类，
//! 两块硬盘里一块是系统盘；分享给了一位好友。另一个应用一份分享都没有。没有后端，别的应用的详情照旧取不到；
//! 这两个写死，界面测试看“设备”和“分享的家人”两节怎么画。

use wonton_protocol::{
    Gate, InstalledPlugin, PluginDetail, PluginDetailRoute, PluginDeviceKind, PluginFailure,
    PluginGrantedDevice, PluginProblem, PluginRuntime, PluginShares, PluginState, ShareGrant,
    ShareRecipient,
};

pub(super) const HUB_TITLE: &str = "Home Hub";
pub(super) const HUB_ID: &str = "app.example.hub";
/// 另一个假应用：详情页上一个人都没分享，看得到“添加分享”。
pub(super) const BUDGET_TITLE: &str = "Budget";
pub(super) const BUDGET_ID: &str = "app.example.budget";

/// 网格上的那一格：串口不在了，状态是异常。
pub(super) fn status() -> PluginState {
    PluginState::Degraded {
        reason: GONE.into(),
    }
}

const GONE: &str = "Sonoff Zigbee 3.0 is not connected. Plug it back in.";

/// 这条路径是那个带设备的假应用时是它的详情；别的路径为空，照常走没有后端的那条路。
pub(super) fn detail(
    route: &PluginDetailRoute,
    installed: Option<InstalledPlugin>,
) -> Option<PluginDetail> {
    let PluginDetailRoute::Installed { id } = route else {
        return None;
    };
    match id.as_str() {
        HUB_ID => Some(hub(installed)),
        BUDGET_ID => Some(budget(installed)),
        _ => None,
    }
}

fn budget(installed: Option<InstalledPlugin>) -> PluginDetail {
    let mut detail = app(
        BUDGET_ID,
        BUDGET_TITLE,
        "What the household spent this month",
        installed,
    );
    detail.has_web = true;
    detail.shares = Some(PluginShares {
        per_user_accounts: false,
        friend_count: 0,
        grants: Vec::new(),
    });
    detail
}

fn app(id: &str, title: &str, summary: &str, installed: Option<InstalledPlugin>) -> PluginDetail {
    PluginDetail {
        source: None,
        id: id.into(),
        title: title.into(),
        summary: summary.into(),
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
        problems: Vec::new(),
        devices: Vec::new(),
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

fn hub(installed: Option<InstalledPlugin>) -> PluginDetail {
    let mut detail = app(
        HUB_ID,
        HUB_TITLE,
        "Zigbee, USB scanners and disk health in one place",
        installed,
    );
    detail.has_web = true;
    detail.problems = vec![PluginProblem {
        reason: PluginFailure::DeviceUnavailable,
        message: GONE.into(),
        min_version: None,
        settings: None,
    }];
    detail.devices = vec![
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
    ];
    // 分享区：已经分享给一位好友，详情页上看得到状态和那个人。
    detail.shares = Some(PluginShares {
        per_user_accounts: false,
        friend_count: 1,
        grants: vec![ShareGrant {
            id: "grant-hub-bo".into(),
            plugin_id: HUB_ID.into(),
            recipient: ShareRecipient::Friend {
                user_id: "@bo:home.example".into(),
            },
            expires_at_ms: None,
        }],
    });
    detail
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
