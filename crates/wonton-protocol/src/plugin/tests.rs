use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;
use crate::Action;

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

fn permission(permission: &str, text: &str, high: bool) -> PermissionLine {
    PermissionLine {
        permission: permission.into(),
        text: text.into(),
        high,
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
        kind: PluginKind::App,
        category: Some(PluginCategory::Photos),
        icon: Some(icon("ab")),
        trust: PluginTrust::Official,
        source: Some("github:example/wonton-immich@v2.0.0".into()),
        status: PluginState::Running {
            health: PluginHealth::Healthy,
        },
        web_url: Some("http://app-immich-photos.wonton.localhost:8480".into()),
        bot_user_id: None,
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
        category: Some(PluginCategory::Photos),
        tags: vec!["immich".into(), "照片".into()],
        kind: PluginKind::App,
        version: "2.1.0".into(),
        trust: PluginTrust::Official,
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
        },
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
                message: "连不上 GitHub".into(),
            }),
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
        author: None,
        permissions: vec![
            permission(
                "storage.share.documents:write",
                "读写共享文件夹 documents",
                false,
            ),
            permission("network.host", "使用宿主的网络（高权限）", true),
        ],
        notices: vec![CardLine {
            text: "云吞没有审核过这个插件，代码和镜像由 someone 提供".into(),
            emphasis: true,
        }],
        upgrade_from: None,
        release_notes: None,
        blocked: None,
    }
}

fn detail() -> PluginDetail {
    PluginDetail {
        source: Some("github:example/wonton-immich".into()),
        id: "app.immich.photos".into(),
        title: "相册".into(),
        summary: "照片备份、人脸与语义搜索".into(),
        about: Some("第一段\n\n第二段".into()),
        icon: Some(icon("ab")),
        screenshots: vec![PluginImage {
            path: "/_wonton/api/images/ef/screenshot.jpg".into(),
        }],
        category: Some(PluginCategory::Photos),
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
            false,
        )],
        requirements: vec!["需要 2 GB 内存".into()],
        has_web: true,
        installed: Some(installed()),
        blocked: None,
    }
}

/// 契约：核心的接口和 ViewModel 用到的类型都能经 JSON 往返。
#[test]
fn interface_types_round_trip_through_json() {
    let installed = InstalledPlugins {
        plugins: vec![installed()],
        upgrade_check: Some(StoreProblem {
            reason: StoreFailure::RateLimited,
            message: "GitHub 限速了".into(),
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
    };
    assert_eq!(round_trip(&request), request);
    let upgrade = UpgradeRequest {
        digest: preview().digest,
        grant: vec![],
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
        }],
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
        Action::RefreshStore { force: true },
        Action::InstallPlugin { preview: preview() },
        Action::UpgradePlugin {
            preview: InstallPreview {
                upgrade_from: Some("0.9.0".into()),
                ..preview()
            },
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
        serde_json::to_value(PluginState::Running {
            health: PluginHealth::Healthy
        })
        .unwrap(),
        json!({"state": "running", "health": "healthy"})
    );
    assert_eq!(
        serde_json::to_value(PluginState::Degraded { reason: "x".into() }).unwrap(),
        json!({"state": "degraded", "reason": "x"})
    );
    assert_eq!(
        serde_json::to_value(PluginCategory::Ai).unwrap(),
        json!("ai")
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
    assert_eq!(
        serde_json::to_value(PluginTaskState::Failed {
            reason: PluginFailure::SourceUnavailable,
            message: "x".into(),
        })
        .unwrap(),
        json!({"type": "failed", "reason": "source_unavailable", "message": "x"})
    );
}

/// 核心可能比 App 新：不认识的取值落到兜底的变体，多出来的字段被忽略，整份列表照样能读。
#[test]
fn values_from_a_newer_core_still_parse() {
    let parse = |value| serde_json::from_value::<StoreItem>(value).unwrap();
    let mut item = serde_json::to_value(&store().official.plugins[0]).unwrap();
    item["category"] = json!("gardening");
    item["kind"] = json!("widget");
    item["trust"] = json!("verified");
    item["added_in_a_later_version"] = json!(true);
    let item = parse(item);
    assert_eq!(item.category, Some(PluginCategory::Other));
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
}

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}
