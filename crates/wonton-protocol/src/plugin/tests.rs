use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;
use crate::{Action, LanPort, LanProtocol, TabLayout};

fn icon(hash: &str) -> PluginIcon {
    PluginIcon {
        small: PluginImage {
            path: format!("/_wonton/api/images/{hash}/icon-192.png"),
        },
        large: PluginImage {
            path: format!("/_wonton/api/images/{hash}/icon-512.png"),
        },
    }
}

fn permission(permission: &str, text: &str, risk: Option<PermissionRisk>) -> PermissionLine {
    PermissionLine {
        permission: permission.into(),
        text: text.into(),
        risk,
        notice: None,
        supported: true,
        added: false,
    }
}

fn installed() -> InstalledPlugin {
    InstalledPlugin {
        id: "app.immich.photos".into(),
        title: "相册".into(),
        summary: "照片备份、人脸与语义搜索".into(),
        version: "2.0.0".into(),
        revision: "opaque:revision/版本-2".into(),
        kind: PluginKind::App,
        icon: Some(icon("ab")),
        trust: PluginTrust::Official,
        source: Some("github:example/wonton-immich@v2.0.0".into()),
        provider: Some("github.com/example/wonton-immich".into()),
        via_index: None,
        written_by: Some(PluginWriter {
            plugin_id: "app.hermes".into(),
            title: "Hermes".into(),
        }),
        status: PluginState::Running {
            health: PluginHealth::Healthy,
        },
        web_url: Some("http://app-immich-photos.wonton.localhost:8480".into()),
        web_display: TabDisplay::Fullscreen,
        cross_origin_isolated: false,
        declares_tools: true,
        tabs: vec![PluginTab {
            id: "photos".into(),
            title: "照片".into(),
            icon: PluginImage {
                path: "/_wonton/api/images/photos.png".into(),
            },
            icon_selected: None,
            path: "/photos".into(),
            layout: TabLayout::EdgeToEdge,
            display: TabDisplay::Standard,
        }],
        bot_user_id: None,
        family: false,
        upgrade: Some(UpgradeOffer {
            version: "2.1.0".into(),
            release_notes: Some("支持视频转码".into()),
            permissions_changed: true,
        }),
        removable: true,
        notices: vec![],
    }
}

fn store() -> StoreListing {
    let item = StoreItem {
        source: "github:example/wonton-immich".into(),
        id: "app.immich.photos".into(),
        title: "相册".into(),
        summary: "照片备份、人脸与语义搜索".into(),
        icon: Some(icon("ab")),
        tags: vec!["immich".into(), "照片".into()],
        kind: PluginKind::App,
        version: "2.1.0".into(),
        trust: PluginTrust::Official,
        provider: Some("github.com/example/wonton-immich".into()),
        via_index: None,
        author: Some(PluginAuthor {
            name: "Example".into(),
            url: Some("https://example.com".into()),
        }),
        stars: None,
        installed_version: Some("2.0.0".into()),
        high_permissions: false,
        blocked: None,
    };
    StoreListing {
        official: StoreSection {
            plugins: vec![item.clone()],
            fetched_at_ms: Some(1_793_000_000_000),
            problem: None,
            unreadable: Vec::new(),
        },
        added: vec![AddedSource {
            address: "https://example.com/index.json".into(),
            kind: SourceKind::Index,
            provider: "example.com".into(),
            name: Some("Example 的插件".into()),
            section: StoreSection {
                plugins: vec![StoreItem {
                    source: "index+https://example.com/index.json#app.immich.photos".into(),
                    trust: PluginTrust::Community,
                    via_index: Some("example.com".into()),
                    ..item.clone()
                }],
                fetched_at_ms: Some(1_793_000_000_000),
                problem: None,
                unreadable: Vec::new(),
            },
        }],
        community: StoreSection {
            plugins: vec![StoreItem {
                source: "github:someone/wonton-samba".into(),
                trust: PluginTrust::Community,
                stars: Some(42),
                high_permissions: true,
                blocked: Some("宿主是 arm64，这个插件只支持 amd64".into()),
                ..item
            }],
            fetched_at_ms: None,
            problem: Some(StoreProblem {
                reason: StoreFailure::Unreachable,
                details: "连不上 GitHub".into(),
            }),
            unreadable: vec![StoreUnreadable {
                source: "github:acme/old".into(),
                reason: "unknown field `category`".into(),
            }],
        },
    }
}

fn preview() -> InstallPreview {
    InstallPreview {
        source: Some("github:someone/wonton-samba@v1.0.0".into()),
        commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
        digest: format!("sha256:{}", "cd".repeat(32)),
        id: "app.samba".into(),
        title: "Samba".into(),
        summary: "局域网文件共享".into(),
        version: "1.0.0".into(),
        kind: PluginKind::App,
        icon: None,
        trust: PluginTrust::Community,
        provider: Some("github.com/someone/wonton-samba".into()),
        via_index: None,
        written_by: Some(PluginWriter {
            plugin_id: "app.hermes".into(),
            title: "Hermes".into(),
        }),
        author: None,
        permissions: vec![
            permission(
                "storage.share.documents:write",
                "读写共享文件夹 documents",
                None,
            ),
            permission(
                "network.host",
                "使用宿主的网络（很高权限）",
                Some(PermissionRisk::VeryHigh),
            ),
        ],
        notices: vec![CardLine {
            text: "云吞没有审核过这个应用，代码和镜像由 someone 提供".into(),
            emphasis: true,
        }],
        upgrade_from: None,
        release_notes: None,
        missing_profile: None,
        blocked: None,
        devices: Some(vec![PluginDevice {
            id: "usb-1".into(),
            name: "Zigbee 棒".into(),
            target: "/dev/zigbee".into(),
            service: "homeassistant".into(),
            optional: true,
        }]),
        lan_webui: Some(LanWebuiOffer {
            ports: vec![LanPort {
                port: 8080,
                protocol: LanProtocol::Tcp,
            }],
            risk: "局域网里的设备，以及这台主机上能出网的插件，不用密码就能以你的身份打开它".into(),
            enabled: false,
            client: None,
            client_problem: None,
        }),
        always_allowed: vec![BotSwitch {
            caller: "app.hermes".into(),
            name: "Hermes".into(),
            enabled: true,
        }],
    }
}

fn detail() -> PluginDetail {
    PluginDetail {
        source: Some("github:example/wonton-immich".into()),
        provider: Some("github.com/example/wonton-immich".into()),
        via_index: None,
        id: "app.immich.photos".into(),
        title: "相册".into(),
        summary: "照片备份、人脸与语义搜索".into(),
        about: Some("第一段\n\n第二段".into()),
        icon: Some(icon("ab")),
        screenshots: vec![PluginImage {
            path: "/_wonton/api/images/ef/screenshot.jpg".into(),
        }],
        tags: vec![],
        kind: PluginKind::App,
        version: "2.1.0".into(),
        trust: PluginTrust::Official,
        author: None,
        homepage: Some("https://immich.app".into()),
        repository: Some("https://github.com/example/wonton-immich".into()),
        license: Some("AGPL-3.0-only".into()),
        stars: None,
        release_notes: None,
        permissions: vec![permission(
            "storage.share.photos:read",
            "读共享文件夹 photos",
            None,
        )],
        requirements: vec![RequirementLine {
            text: "需要 2 GB 内存".into(),
            status: RequirementStatus::Unmet,
            problem: Some("这台主机只有 1.5 GB 内存".into()),
        }],
        has_web: true,
        installed: Some(installed()),
        blocked: None,
        notices: Vec::new(),
        lan_webui: None,
        save_grants: Vec::new(),
        ask_every_update: Some(false),
        limits: Some(PluginLimits {
            memory: Some(4 * 1024 * 1024 * 1024),
            cpu_percent: None,
        }),
    }
}

/// 契约：核心的接口和 ViewModel 用到的类型都能经 JSON 往返。
#[test]
fn interface_types_round_trip_through_json() {
    let installed = InstalledPlugins {
        plugins: vec![installed()],
        upgrade_check: Some(StoreProblem {
            reason: StoreFailure::RateLimited,
            details: "GitHub 限速了".into(),
        }),
    };
    assert_eq!(round_trip(&installed), installed);
    assert_eq!(round_trip(&store()), store());
    assert_eq!(round_trip(&detail()), detail());
    assert_eq!(round_trip(&preview()), preview());

    let source = InstallSource {
        source: "github:someone/wonton-samba".into(),
    };
    assert_eq!(round_trip(&source), source);
    let request = InstallRequest {
        source: "github:someone/wonton-samba@v1.0.0".into(),
        digest: preview().digest,
        grant: vec!["network.host".into()],
        devices: vec!["usb-1".into()],
        lan_webui: true,
    };
    assert_eq!(round_trip(&request), request);
    let upgrade = UpgradeRequest {
        devices: vec!["serial-1".into()],
        source: Some("upload:sha256:abc".into()),
        digest: preview().digest,
        grant: vec![],
        lan_webui: false,
    };
    assert_eq!(round_trip(&upgrade), upgrade);

    let view_model = PluginsViewModel {
        installed: Some(installed),
        store: Some(store()),
        refreshing: true,
        problem: Some(PluginProblem {
            reason: PluginFailure::TransportUnavailable,
            message: "连不上家里的服务器".into(),
        }),
        tasks: vec![PluginTask {
            id: "app.samba".into(),
            title: "Samba".into(),
            icon: None,
            operation: PluginOperation::Install,
            state: PluginTaskState::Failed {
                reason: PluginFailure::Changed,
                message: "插件包在预览之后变了".into(),
            },
            added_tools: Some(AddedTools::NotCompared {
                reason: "插件没有运行，列不出工具".into(),
            }),
        }],
        egress: EgressViewModel::default(),
        registry_mirror: crate::RegistryMirrorViewModel::default(),
        core_upgrade: crate::CoreUpgradeViewModel::default(),
        storage: Some(StorageUsage {
            total_bytes: 500,
            free_bytes: 200,
        }),
        storage_problem: None,
        takeover: Some(PluginTakeover {
            id: "app.memos".into(),
            state: TakeoverState::Lost {
                problem: PluginProblem {
                    reason: PluginFailure::TransportUnavailable,
                    message: "连不上家里的服务器".into(),
                },
            },
        }),
    };
    assert_eq!(round_trip(&view_model), view_model);
}

#[test]
fn plugin_actions_round_trip_through_json() {
    for action in [
        Action::SetLanguages {
            languages: vec!["zh-Hans-CN".into(), "en".into()],
        },
        Action::RefreshPlugins,
        Action::StartPlugin {
            plugin_id: "app.samba".into(),
        },
        Action::RefreshStore { force: true },
        Action::InstallPlugin { preview: preview() },
        Action::UpgradePlugin {
            preview: InstallPreview {
                upgrade_from: Some("0.9.0".into()),
                ..preview()
            },
        },
        Action::StartTakeover {
            id: "app.memos".into(),
        },
        Action::EndTakeover {
            id: "app.memos".into(),
        },
        Action::UninstallPlugin {
            id: "app.samba".into(),
            delete_data: true,
        },
        Action::DismissPluginTask {
            id: "app.samba".into(),
        },
    ] {
        assert_eq!(round_trip(&action), action);
    }
}

/// 契约：核心和客户端核心之间的 JSON 写法。改了就要两边一起改，旧的 App 也会读不懂。
#[test]
fn wire_format_is_stable() {
    assert_eq!(
        serde_json::to_value(installed()).unwrap()["revision"],
        json!("opaque:revision/版本-2")
    );
    assert_eq!(
        serde_json::to_value(PluginLimits {
            memory: Some(1024),
            cpu_percent: Some(150),
        })
        .unwrap(),
        json!({"memory": 1024, "cpu_percent": 150})
    );
    assert_eq!(
        serde_json::from_value::<PluginLimits>(json!({})).unwrap(),
        PluginLimits::default()
    );
    assert_eq!(
        serde_json::to_value(PluginState::Running {
            health: PluginHealth::Healthy
        })
        .unwrap(),
        json!({"state": "running", "health": "healthy"})
    );
    assert_eq!(
        serde_json::to_value(PluginState::Stopped).unwrap(),
        json!({"state": "stopped"})
    );
    assert_eq!(round_trip(&PluginState::Stopped), PluginState::Stopped);
    assert_eq!(
        serde_json::to_value(PluginState::Degraded { reason: "x".into() }).unwrap(),
        json!({"state": "degraded", "reason": "x"})
    );
    assert_eq!(
        serde_json::to_value(RequirementStatus::Degraded).unwrap(),
        json!("degraded")
    );
    assert_eq!(
        serde_json::from_value::<RequirementStatus>(json!("later")).unwrap(),
        RequirementStatus::Unknown
    );
    assert_eq!(
        serde_json::to_value(PluginFailure::CoreUpgrading).unwrap(),
        json!("core_upgrading")
    );
    assert_eq!(
        serde_json::to_value(PluginTrust::Community).unwrap(),
        json!("community")
    );
    assert_eq!(
        serde_json::to_value(StoreFailure::RateLimited).unwrap(),
        json!("rate_limited")
    );
    let request: InstallRequest = serde_json::from_value(json!({
        "source": "github:o/r@v1.0.0",
        "digest": "sha256:00",
        "grant": ["models"],
    }))
    .unwrap();
    assert_eq!(request.grant, ["models"]);
    assert!(request.devices.is_empty(), "旧 App 不带设备选择，按空算");
    assert!(!request.lan_webui, "旧 App 不带这个字段，按关算");
    assert_eq!(
        serde_json::to_value(PluginTaskState::Failed {
            reason: PluginFailure::SourceUnavailable,
            message: "x".into(),
        })
        .unwrap(),
        json!({"type": "failed", "reason": "source_unavailable", "message": "x"})
    );
    assert_eq!(
        serde_json::to_value(PluginFailure::EgressProxyUnavailable).unwrap(),
        json!("egress_proxy_unavailable")
    );
    assert_eq!(
        serde_json::to_value(PluginFailure::DeviceUnavailable).unwrap(),
        json!("device_unavailable")
    );
}

/// 权限风险字段兼容旧 App，并把未知级别落到 `Other`。
#[test]
fn permission_risk_keeps_unknown_values_safe() {
    let old_line: PermissionLine = serde_json::from_value(json!({
        "permission": "models",
        "text": "使用模型网关",
        "supported": true,
        "added": false,
    }))
    .unwrap();
    assert_eq!(old_line.risk, None);
    let future_line: PermissionLine = serde_json::from_value(json!({
        "permission": "network.host",
        "text": "使用宿主的网络",
        "risk": "critical",
        "supported": true,
        "added": false,
    }))
    .unwrap();
    assert_eq!(future_line.risk, Some(PermissionRisk::Other));
}

#[test]
fn permission_risk_and_notice_keep_wire_values() {
    let line = PermissionLine {
        permission: "docker.socket".into(),
        text: "访问 Docker socket".into(),
        risk: Some(PermissionRisk::Highest),
        notice: Some("这个插件能控制整台主机".into()),
        supported: true,
        added: true,
    };
    assert_eq!(round_trip(&line), line);
    assert_eq!(
        serde_json::to_value(PermissionRisk::VeryHigh).unwrap(),
        json!("very_high")
    );
}
#[test]
fn an_uncertain_task_keeps_its_wire_format() {
    assert_eq!(
        serde_json::to_value(PluginTaskState::Uncertain {
            message: "x".to_owned(),
        })
        .unwrap(),
        json!({"type": "uncertain", "message": "x"})
    );
}

#[test]
fn the_ask_every_update_switch_keeps_its_wire_format() {
    let switch = AskEveryUpdate { enabled: true };
    assert_eq!(json!(switch), json!({"enabled": true}));
    assert_eq!(round_trip(&switch), switch);
}

#[test]
fn a_core_without_writers_lists_plugins_without_them() {
    let mut item = serde_json::to_value(installed()).unwrap();
    item.as_object_mut().unwrap().remove("written_by");
    let item: InstalledPlugin = serde_json::from_value(item).unwrap();
    assert_eq!(item.written_by, None);

    let mut shown = serde_json::to_value(detail()).unwrap();
    shown.as_object_mut().unwrap().remove("ask_every_update");
    let shown: PluginDetail = serde_json::from_value(shown).unwrap();
    assert_eq!(shown.ask_every_update, None);
}

#[test]
fn a_core_without_tabs_lists_plugins_without_pages() {
    let mut plugin = serde_json::to_value(installed()).unwrap();
    let fields = plugin.as_object_mut().unwrap();
    fields.remove("tabs");
    fields.remove("web_display");
    fields.remove("cross_origin_isolated");
    let plugin: InstalledPlugin = serde_json::from_value(plugin).unwrap();
    assert!(plugin.tabs.is_empty());
    assert_eq!(plugin.web_display, TabDisplay::Standard);
    assert!(!plugin.cross_origin_isolated);
}

#[test]
fn values_from_a_newer_core_still_parse() {
    let parse = |value| serde_json::from_value::<StoreItem>(value).unwrap();
    let mut item = serde_json::to_value(&store().official.plugins[0]).unwrap();
    item["kind"] = json!("widget");
    item["trust"] = json!("verified");
    item["added_in_a_later_version"] = json!(true);
    let item = parse(item);
    assert_eq!(item.kind, PluginKind::Other);
    assert_eq!(item.trust, PluginTrust::Unknown);

    let state: PluginState = serde_json::from_value(json!({"state": "hibernating"})).unwrap();
    assert_eq!(state, PluginState::Unknown);
    let state: PluginState =
        serde_json::from_value(json!({"state": "running", "health": "degrading"})).unwrap();
    assert_eq!(
        state,
        PluginState::Running {
            health: PluginHealth::Unknown
        }
    );
    let failure: StoreFailure = serde_json::from_value(json!("quota_exceeded")).unwrap();
    assert_eq!(failure, StoreFailure::Other);

    let mut plugin = serde_json::to_value(installed()).unwrap();
    plugin["web_display"] = json!("picture_in_picture");
    plugin["tabs"][0]["layout"] = json!("sidebar");
    plugin["tabs"][0]["display"] = json!("picture_in_picture");
    let plugin: InstalledPlugin = serde_json::from_value(plugin).unwrap();
    assert_eq!(plugin.web_display, TabDisplay::Other);
    assert_eq!(plugin.tabs[0].layout, TabLayout::Other);
    assert_eq!(plugin.tabs[0].display, TabDisplay::Other);
}

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}
