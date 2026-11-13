//! 一键装/升（ADR-0017）和侧边栏、底栏的接口契约：线上 JSON 写法和新 App 字段。

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;
use crate::{Action, EgressViewModel, PluginState, StoreConfirmation, TabRef, UpdatablePlugin};

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

/// 商店条目的 `high_permissions` 是核心给的：旧核心的列表没有这个字段，按没有算，壳不自己比档位。
#[test]
fn store_items_without_high_permissions_default_to_no_tag() {
    let mut item = serde_json::to_value(store()).unwrap();
    let official = item
        .pointer_mut("/official/plugins/0")
        .unwrap()
        .as_object_mut()
        .unwrap();
    assert_eq!(official["high_permissions"], json!(false));
    official.remove("high_permissions");
    official["security_tier"] = json!("highest");
    let item: StoreItem =
        serde_json::from_value(item.pointer("/official/plugins/0").unwrap().clone()).unwrap();
    assert!(!item.high_permissions);

    let mut item = serde_json::to_value(store()).unwrap();
    let community = item
        .pointer_mut("/community/plugins/0")
        .unwrap()
        .as_object_mut()
        .unwrap();
    assert_eq!(community["high_permissions"], json!(true));
    community.remove("high_permissions");
    let item: StoreItem =
        serde_json::from_value(item.pointer("/community/plugins/0").unwrap().clone()).unwrap();
    assert!(!item.high_permissions);
}

/// 契约：一键装/升的操作、安装请求和升级报价的 JSON 写法；旧 App 读得懂新 App 加的字段。
#[test]
fn one_tap_requests_round_trip() {
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
        plan: "consumer-plan".into(),
        scope: None,
        grant: vec!["network.host".into()],
        enabled_permissions: vec!["docker.socket".into()],
        secrets: SecretValues::from([("brave_key".into(), crate::SecretString::new("s3cret"))]),
        settings: SettingValues::from([("months".into(), SettingValue::Integer { value: 6 })]),
        devices: vec!["usb-1".into()],
        lan_webui: true,
        install_together: vec![InstallTogether {
            id: "app.model.gateway".into(),
            request: InstallRequest {
                source: "github:example/wonton-model-gateway".into(),
                digest: format!("sha256:{}", "ab".repeat(32)),
                plan: "provider-plan".into(),
                scope: None,
                grant: Vec::new(),
                enabled_permissions: Vec::new(),
                secrets: SecretValues::new(),
                settings: SettingValues::new(),
                devices: Vec::new(),
                lan_webui: false,
                install_together: Vec::new(),
                connect_plugins: Vec::new(),
            },
        }],
        connect_plugins: Vec::new(),
    };
    assert_eq!(round_trip(&request), request);
    let upgrade = UpgradeRequest {
        devices: vec!["serial-1".into()],
        source: Some("upload:sha256:abc".into()),
        digest: preview().digest,
        grant: vec![],
        enabled_permissions: vec![],
        secrets: SecretValues::new(),
        settings: SettingValues::new(),
        lan_webui: false,
    };
    assert_eq!(round_trip(&upgrade), upgrade);
}

/// 契约：侧边栏 ViewModel（插件列表 + 侧边栏）和商店列表在新字段下的往返。
#[test]
fn sidebar_view_model_round_trip() {
    let installed = InstalledPlugins {
        plugins: vec![installed()],
        upgrade_check: None,
    };
    let view_model = PluginsViewModel {
        installed: Some(installed),
        roles: Default::default(),
        store: Some(store()),
        refreshing: true,
        problem: Some(PluginProblem {
            reason: PluginFailure::TransportUnavailable,
            min_version: None,
            message: "连不上家里的服务器".into(),
            settings: None,
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
            waiting_for_connection: false,
            added_tools: Some(AddedTools::NotCompared {
                reason: "插件没有运行，列不出工具".into(),
            }),
        }],
        updatable: vec![UpdatablePlugin {
            plugin_id: "app.memos".into(),
            title: "Memos".into(),
            version: "0.25.0".into(),
            icon: None,
        }],
        pending: vec!["app.memos".into()],
        confirmation: Some(StoreConfirmation::Upgrade { preview: preview() }),
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
                    min_version: None,
                    message: "连不上家里的服务器".into(),
                    settings: None,
                },
            },
        }),
        actions: Vec::new(),
        shared_apps: None,
        port_mappings: Vec::new(),
        owner_secrets: None,
        media_grants: Vec::new(),
        sidebar: crate::SidebarViewModel {
            apps: vec![crate::SidebarApp {
                plugin_id: Some("app.memos".into()),
                tab: Some(TabRef::Plugin {
                    plugin_id: "app.memos".into(),
                    page: None,
                }),
                title: "Memos".into(),
                open: crate::SidebarOpen::Page {
                    tab: TabRef::Plugin {
                        plugin_id: "app.memos".into(),
                        page: None,
                    },
                },
                icon: None,
                badge: crate::SidebarBadge {
                    update: true,
                    problem: false,
                },
            }],
            more_apps: false,
            updates: 1,
            settings_badge: false,
        },
    };
    assert_eq!(round_trip(&view_model), view_model);
}

/// 契约：核心和客户端核心之间的 JSON 写法。改了就要两边一起改，旧的 App 也会读不懂。
#[test]

fn wire_format_is_stable() {
    for action in [
        Action::InstallNow {
            source: "github:owner/repo".into(),
        },
        Action::UpgradeNow {
            plugin_id: "app.memos".into(),
        },
        Action::UpgradeAll,
        Action::DismissStoreConfirmation,
    ] {
        assert_eq!(round_trip(&action), action);
    }
    assert_eq!(
        serde_json::to_value(Action::UpgradeNow {
            plugin_id: "app.memos".into()
        })
        .unwrap(),
        json!({"type": "upgrade_now", "plugin_id": "app.memos"})
    );
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
    assert_eq!(
        serde_json::to_value(PluginState::NeedsConfiguration).unwrap(),
        json!({"state": "needs_configuration"})
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
        serde_json::to_value(PluginFailure::RecordUnusable).unwrap(),
        json!("record_unusable")
    );
    assert_eq!(
        serde_json::to_value(PluginTrust::Community).unwrap(),
        json!("community")
    );
    assert_eq!(
        serde_json::to_value(StoreFailure::RateLimited).unwrap(),
        json!("rate_limited")
    );
}

/// 契约：旧 App 发的安装请求缺新字段时的默认值。
#[test]
fn older_app_install_defaults() {
    let request: InstallRequest = serde_json::from_value(json!({
        "source": "github:o/r@v1.0.0",
        "digest": "sha256:00",
        "grant": ["models"],
        "secrets": {"brave_key": "s3cret"},
    }))
    .unwrap();
    assert_eq!(request.grant, ["models"]);
    assert_eq!(request.secrets["brave_key"].expose(), "s3cret");
    assert!(request.devices.is_empty(), "旧 App 不带设备选择，按空算");
    assert!(!request.lan_webui, "旧 App 不带这个字段，按关算");
    assert!(
        request.install_together.is_empty(),
        "旧 App 不带一起装的插件，按空算"
    );

    let older: InstallRequest = serde_json::from_value(json!({
        "source": "github:o/r@v1.0.0",
        "digest": "sha256:00",
        "grant": ["models"],
    }))
    .unwrap();
    assert!(older.secrets.is_empty(), "旧 App 不带密钥表，按没填算");
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
