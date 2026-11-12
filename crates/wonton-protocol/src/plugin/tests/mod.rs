mod actions;
mod forms;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;
use crate::plugin::hint::{PluginHint, PluginHintCategory};
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
        optional: false,
        enabled: false,
        off_loss: None,
        opened: None,
    }
}

fn installed() -> InstalledPlugin {
    InstalledPlugin {
        chat: Default::default(),
        id: "app.immich.photos".into(),
        title: "相册".into(),
        summary: "照片备份、人脸与语义搜索".into(),
        version: "2.0.0".into(),
        revision: "opaque:revision/版本-2".into(),
        kind: PluginKind::App,
        runtime: PluginRuntime::Compose,
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
        enabled_root_permissions: vec!["docker.socket".into()],
        install_progress: None,
        tunnel_ports: Vec::new(),
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
        security_tier: Some(SecurityTier::Standard),
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
                security_tier: Some(SecurityTier::Highest),
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
        plan: "consumer-plan".into(),
        id: "app.samba".into(),
        title: "Samba".into(),
        summary: "局域网文件共享".into(),
        version: "1.0.0".into(),
        kind: PluginKind::App,
        runtime: PluginRuntime::Compose,
        security_tier: Some(SecurityTier::High),
        scope_choices: vec![],
        has_bot: false,
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
        secret_inputs: vec![SecretInput {
            name: "brave_key".into(),
            title: "Brave Search API key".into(),
            description: Some("Free tier has 2,000 queries a month".into()),
            url: Some("https://api-dashboard.search.brave.com/app/keys".into()),
            required: true,
        }],
        settings: None,
        notices: vec![CardLine {
            text: "云吞没有审核过这个应用，代码和镜像由 someone 提供".into(),
            emphasis: true,
        }],
        missing_requires: vec![MissingCapability {
            capability: "jellyfin".into(),
            third_party: false,
            providers: vec![CapabilityProvider {
                id: "app.jellyfin".into(),
                title: "Jellyfin".into(),
                source: "github:example/wonton-jellyfin".into(),
                high_privilege: false,
            }],
        }],
        missing_wants: vec![MissingCapability {
            capability: "x-acme.ledger".into(),
            third_party: true,
            providers: Vec::new(),
        }],
        wanting_plugins: Vec::new(),
        upgrade_from: None,
        release_notes: None,
        missing_profile: None,
        blocked: None,
        blocked_category: None,
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
        runtime: PluginRuntime::Compose,
        security_tier: Some(SecurityTier::Standard),
        link_url: None,
        scope: None,
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
        secret_inputs: vec![SecretInputState {
            name: "brave_key".into(),
            title: "Brave Search API key".into(),
            required: false,
            filled: true,
            changed_at_ms: Some(1_793_000_000_000),
        }],
        has_web: true,
        has_settings: false,
        installed: Some(installed()),
        blocked: None,
        notices: Vec::new(),
        hints: vec![PluginHint {
            category: PluginHintCategory::ModelsNotConnected,
            detail: Some("模型网关没在运行".into()),
            provider: None,
            capability: None,
        }],
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        capability_providers: vec![ConsumerCapabilitySetting {
            capability: "jellyfin".into(),
            third_party: false,
            providers: vec![CapabilityProviderRef {
                id: "app.jellyfin".into(),
                title: "Jellyfin".into(),
                high_privilege: false,
            }],
            selected: None,
        }],
        lan_webui: None,
        save_grants: Vec::new(),
        ask_every_update: Some(false),
        limits: Some(PluginLimits {
            memory: Some(4 * 1024 * 1024 * 1024),
            cpu_percent: None,
        }),
        shares: None,
    }
}

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
        optional: true,
        enabled: true,
        off_loss: Some("看不到每个容器的占用".into()),
        opened: Some(PermissionOpened {
            since: 1_760_000_000,
            bot: Some(PluginWriter {
                plugin_id: "app.hermes".into(),
                title: "Hermes".into(),
            }),
        }),
    };
    assert_eq!(round_trip(&line), line);
    assert_eq!(
        serde_json::to_value(&line).unwrap()["opened"]["bot"]["plugin_id"],
        json!("app.hermes")
    );
    let update = PermissionUpdate { enabled: true };
    assert_eq!(round_trip(&update), update);
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
            next_step: PluginTaskNextStep::CheckInstalled,
        })
        .unwrap(),
        json!({"type": "uncertain", "message": "x", "next_step": "check_installed"})
    );
}

/// 契约：模型网关没接上的轻提示。类别落在字段上，壳按类别写文字（ADR-0043）；核心写的只有 `detail`。
#[test]
fn the_models_not_connected_hint_keeps_its_wire_format() {
    let hint = PluginHint {
        category: PluginHintCategory::ModelsNotConnected,
        detail: Some("网关拒绝了凭据命令（issue，退出码 1）".into()),
        provider: None,
        capability: None,
    };
    assert_eq!(
        json!(hint),
        json!({
            "category": "models_not_connected",
            "detail": "网关拒绝了凭据命令（issue，退出码 1）",
            "provider": null, "capability": null,
        })
    );
    assert_eq!(round_trip(&hint), hint);

    let quiet = PluginHint {
        category: PluginHintCategory::ModelsNotConnected,
        detail: None,
        provider: None,
        capability: None,
    };
    assert_eq!(
        json!(quiet),
        json!({"category": "models_not_connected", "detail": null, "provider": null, "capability": null})
    );

    let mut shown = serde_json::to_value(detail()).unwrap();
    shown["hints"] = json!([{"category": "models_not_connected"}]);
    let shown: PluginDetail = serde_json::from_value(shown).unwrap();
    assert_eq!(
        shown.hints,
        vec![PluginHint {
            category: PluginHintCategory::ModelsNotConnected,
            detail: None,
            provider: None,
            capability: None,
        }]
    );

    // 网关接上了、但默认路由里没有模型；服务商出错时 detail 带核心写的出错原因。
    let no_default = PluginHint {
        category: PluginHintCategory::ModelsNoDefaultModel,
        detail: None,
        provider: None,
        capability: None,
    };
    let provider_error = PluginHint {
        category: PluginHintCategory::ModelsProviderError,
        detail: Some("服务商 DeepSeek 拒绝了 key（401）".into()),
        provider: None,
        capability: None,
    };
    assert_eq!(
        json!(no_default),
        json!({"category": "models_no_default_model", "detail": null, "provider": null, "capability": null})
    );
    assert_eq!(
        json!(provider_error),
        json!({
            "category": "models_provider_error",
            "detail": "服务商 DeepSeek 拒绝了 key（401）",
            "provider": null, "capability": null,
        })
    );
    assert_eq!(round_trip(&no_default), no_default);
    assert_eq!(round_trip(&provider_error), provider_error);

    let newer: PluginHintCategory = serde_json::from_value(json!("model_pool_drained")).unwrap();
    assert_eq!(newer, PluginHintCategory::Other);
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

    // 老客户端的文档里没有可选权限的字段：落到默认值，也就是“必需、关着”。
    let mut line =
        serde_json::to_value(permission("docker.socket", "访问 Docker socket", None)).unwrap();
    for key in ["optional", "enabled", "off_loss", "opened"] {
        line.as_object_mut().unwrap().remove(key);
    }
    let line: PermissionLine = serde_json::from_value(line).unwrap();
    assert!(!line.optional);
    assert!(!line.enabled);
    assert!(line.off_loss.is_none());
    assert!(line.opened.is_none());

    let mut plugin = serde_json::to_value(installed()).unwrap();
    plugin
        .as_object_mut()
        .unwrap()
        .remove("enabled_root_permissions");
    let plugin: InstalledPlugin = serde_json::from_value(plugin).unwrap();
    assert!(plugin.enabled_root_permissions.is_empty());

    let request: InstallRequest = serde_json::from_value(json!({
        "source": "github:example/wonton-immich",
        "digest": "sha256:abc",
        "grant": [],
    }))
    .unwrap();
    assert!(request.enabled_permissions.is_empty());

    let mut older = serde_json::to_value(preview()).unwrap();
    older.as_object_mut().unwrap().remove("secret_inputs");
    let older: InstallPreview = serde_json::from_value(older).unwrap();
    assert!(
        older.secret_inputs.is_empty(),
        "旧核心没有这个字段，没有要填的"
    );

    let mut shown = serde_json::to_value(detail()).unwrap();
    shown["hints"] = json!([{"category": "model_pool_drained", "detail": "x"}]);
    let shown: PluginDetail = serde_json::from_value(shown).unwrap();
    assert_eq!(
        shown.hints,
        vec![PluginHint {
            category: PluginHintCategory::Other,
            detail: Some("x".into()),
            provider: None,
            capability: None,
        }]
    );
}

#[test]
fn tunnel_ports_from_an_older_or_newer_core_still_parse() {
    let mut older = serde_json::to_value(installed()).unwrap();
    older.as_object_mut().unwrap().remove("tunnel_ports");
    let older: InstalledPlugin = serde_json::from_value(older).unwrap();
    assert!(older.tunnel_ports.is_empty());

    let mut newer = serde_json::to_value(installed()).unwrap();
    newer["tunnel_ports"] = json!([
        {"port": 47998, "protocol": "udp"},
        {"port": 47999, "protocol": "future_transport", "label": "x"},
    ]);
    let newer: InstalledPlugin = serde_json::from_value(newer).unwrap();
    let protocols: Vec<_> = newer.tunnel_ports.iter().map(|p| p.protocol).collect();
    assert_eq!(
        protocols,
        [crate::PortProtocol::Udp, crate::PortProtocol::Other]
    );
}

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

mod capability;
mod install_decisions;
mod legacy_fields;
mod problem;
mod settings;
mod store;
