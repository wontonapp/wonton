//! 缺必需能力的安装（ADR-0053）：确认页写明会一起装的提供方，安装请求把它们的完整决定带上。

use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, PluginTaskState};
use wonton_protocol::{
    CapabilityProvider, CardLine, InstallPreview, MissingCapability, PermissionRisk, PluginAction,
    PluginFailure,
};

use super::rig::{Canned, Rig, WAIT, core_error, preview};

const PROVIDER_SOURCE: &str = "github:owner/app.downloader@v2.0.0";

fn requires(capability: &str) -> MissingCapability {
    MissingCapability {
        capability: capability.to_owned(),
        third_party: false,
        providers: vec![CapabilityProvider {
            id: "app.downloader".to_owned(),
            title: "下载器".to_owned(),
            source: PROVIDER_SOURCE.to_owned(),
            high_privilege: true,
        }],
    }
}

fn consumer_preview() -> InstallPreview {
    let mut consumer = preview("app.photos");
    consumer.missing_requires = vec![requires("torrent_client")];
    consumer
}

fn provider_preview() -> InstallPreview {
    let mut provider = preview("app.downloader");
    provider.source = Some(PROVIDER_SOURCE.to_owned());
    provider.permissions[0].permission = "storage.share.downloads:write".to_owned();
    provider
}

fn answer_previews(rig: &Rig, consumer: &InstallPreview, provider: &InstallPreview) {
    for body in [consumer, provider] {
        rig.core.answer(
            Method::POST,
            "/_wonton/api/plugins/preview",
            Canned(200, serde_json::to_string(body).unwrap()),
        );
    }
}

fn install(rig: &Rig, preview: InstallPreview) {
    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::InstallPlugin {
                preview,
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
}

#[tokio::test]
async fn install_preview_lists_providers_and_permissions() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut provider = provider_preview();
    provider.secret_inputs = vec![wonton_protocol::SecretInput {
        name: "api_key".to_owned(),
        title: "API key".to_owned(),
        description: None,
        url: None,
        required: true,
    }];
    answer_previews(&rig, &consumer_preview(), &provider);

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(shown.install_providers.len(), 1, "缺的必需能力一项一节");
    let section = &shown.install_providers[0];
    assert_eq!(section.capability, "torrent_client");
    assert_eq!(section.providers[0].id, "app.downloader");
    assert!(section.providers[0].high_privilege, "候选要标明高权限");
    let selected = section.selected.as_ref().expect("默认选中第一个提供方");
    assert_eq!(selected.provider.id, "app.downloader");
    assert_eq!(selected.preview.digest, "sha256:abc");
    assert_eq!(selected.preview.permissions[0].text, "读共享文件夹 photos");
    assert_eq!(selected.preview.secret_inputs[0].name, "api_key");
    assert!(
        !wonton_client::install_providers_ready(shown.clone()),
        "必填密钥没填时不能确认"
    );
    let mut filled = shown;
    filled.install_providers[0]
        .selected
        .as_mut()
        .unwrap()
        .secrets
        .insert(
            "api_key".to_owned(),
            wonton_protocol::SecretString::new("s3cret"),
        );
    assert!(wonton_client::install_providers_ready(filled));
}

#[tokio::test]
async fn missing_requires_without_provider_blocks() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = preview("app.photos");
    consumer.missing_requires = vec![MissingCapability {
        capability: "torrent_client".to_owned(),
        third_party: false,
        providers: Vec::new(),
    }];
    answer_previews(&rig, &consumer, &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(shown.install_providers[0].providers.len(), 0);
    assert!(shown.install_providers[0].selected.is_none());
    assert_eq!(
        wonton_client::preview_install_action(shown),
        wonton_client::PluginInstallAction::Blocked
    );
}

#[tokio::test]
async fn wants_missing_does_not_block() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = preview("app.photos");
    consumer.missing_wants = vec![MissingCapability {
        capability: "indexer".to_owned(),
        third_party: false,
        providers: Vec::new(),
    }];
    answer_previews(&rig, &consumer, &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(shown.missing_wants[0].capability, "indexer");
    assert_eq!(
        wonton_client::preview_install_action(shown.clone()),
        wonton_client::PluginInstallAction::Install
    );
    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["install_together"], serde_json::json!([]));
}

#[tokio::test]
async fn install_carries_the_provider_decision_of_the_listed_source() {
    let rig = Rig::new().await;
    rig.connected().await;
    answer_previews(&rig, &consumer_preview(), &provider_preview());
    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();

    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["source"], "github:owner/app.photos@v1.0.0");
    assert_eq!(
        sent["install_together"],
        serde_json::json!([{
            "id": "app.downloader",
            "request": {
                "source": PROVIDER_SOURCE,
                "digest": "sha256:abc",
                "plan": "opaque-install-plan",
                "grant": ["storage.share.downloads:write"],
                "enabled_permissions": [],
                "secrets": {},
                "settings": {},
                "devices": [],
                "lan_webui": false,
                "install_together": [],
                "connect_plugins": [],
            },
        }])
    );
}

#[tokio::test]
async fn a_required_capability_with_no_provider_stops_before_the_request() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = preview("app.photos");
    consumer.missing_requires = vec![MissingCapability {
        capability: "torrent_client".to_owned(),
        third_party: false,
        providers: Vec::new(),
    }];
    answer_previews(&rig, &consumer, &provider_preview());

    install(&rig, consumer);
    let state = rig
        .task_within("app.photos", Duration::from_secs(WAIT.as_secs()), |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert!(
        matches!(
            state,
            PluginTaskState::Failed {
                reason: PluginFailure::Upstream,
                ref message,
            } if message.contains("torrent_client")
        ),
        "{state:?}"
    );
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty()
    );
}

/// 老预览（聊天导入、旧核心）只有候选、没有提供方的完整决定：整单停下，让主人先单独装提供方。
#[tokio::test]
async fn an_old_preview_without_provider_decisions_stops_before_the_request() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = consumer_preview();
    consumer.install_providers.clear();
    install(&rig, consumer);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(
            state,
            PluginTaskState::Failed {
                reason: PluginFailure::Upstream,
                ref message,
            } if message.contains("install the plugin that provides torrent_client")
        ),
        "{state:?}"
    );
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty()
    );
}

/// 提供方带最高级权限：确认页列出并走最高级确认，一起装的请求仍然带上它（ADR-0036、ADR-0053）。
#[tokio::test]
async fn a_provider_with_risky_permissions_is_installed_together() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut provider = provider_preview();
    provider.permissions[0].risk = Some(PermissionRisk::Highest);
    answer_previews(&rig, &consumer_preview(), &provider);

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(
        shown.install_providers[0]
            .selected
            .as_ref()
            .unwrap()
            .preview
            .permissions[0]
            .risk,
        Some(PermissionRisk::Highest),
        "确认页要看得到最高级权限"
    );
    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["install_together"][0]["id"], "app.downloader");
    assert_eq!(
        sent["install_together"][0]["request"]["grant"][0], "storage.share.downloads:write",
        "确认过的提供方权限要随请求带上"
    );
}

/// 一个提供方同时补几个缺的必需能力：只装它一次，确认页也只写一行。
#[tokio::test]
async fn one_provider_covering_two_missing_capabilities_is_installed_once() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = consumer_preview();
    consumer.missing_requires = vec![requires("torrent_client"), requires("indexer")];
    answer_previews(&rig, &consumer, &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(shown.install_providers.len(), 2, "两个能力各一节");
    assert_eq!(
        shown.install_providers[0]
            .selected
            .as_ref()
            .unwrap()
            .provider
            .id,
        "app.downloader"
    );

    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    let together = sent["install_together"].as_array().unwrap();
    assert_eq!(together.len(), 1, "同一个提供方只装一次：{sent:?}");
    assert_eq!(together[0]["id"], "app.downloader");
    // 使用方和提供方各取一次预览，去重后不再为第二个能力重复取。
    assert_eq!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins/preview")
            .len(),
        2
    );
}

/// 上传的包也一样：缺必需能力会一起装提供方，确认页照常写明，安装请求把提供方带上。
#[tokio::test]
async fn an_uploaded_package_preview_also_names_the_provider() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut staged = preview("app.photos");
    staged.source = Some("upload:sha256:abc".to_owned());
    staged.missing_requires = vec![requires("torrent_client")];
    answer_previews(&rig, &staged, &provider_preview());

    let shown = rig
        .app
        .preview_upload(b"\x1f\x8bgz".to_vec())
        .await
        .unwrap();
    let section = shown
        .install_providers
        .first()
        .expect("上传包的确认页也要列出一起装的提供方");
    assert_eq!(section.capability, "torrent_client");
    assert_eq!(section.selected.as_ref().unwrap().provider.title, "下载器");

    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["source"], "upload:sha256:abc");
    assert_eq!(sent["install_together"][0]["id"], "app.downloader");
}

/// 提供方自己 wants 一个已装的高权限能力：预览里的醒目连接提示列在确认页，一起装照样带上它（ADR-0053）。
#[tokio::test]
async fn a_provider_with_a_prominent_connection_notice_is_installed_together() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut provider = provider_preview();
    provider.notices.push(CardLine {
        text: "标题 能连上高权限能力".to_owned(),
        emphasis: true,
    });
    answer_previews(&rig, &consumer_preview(), &provider);

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert!(
        shown.install_providers[0]
            .selected
            .as_ref()
            .unwrap()
            .preview
            .notices
            .iter()
            .any(|notice| notice.emphasis && notice.text.contains("高权限"))
    );
    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["install_together"][0]["id"], "app.downloader");
}

/// 提供方的预览取不到时按真实原因收场：商店源连不上是 Upstream，不归成“输入非法”，
/// 也不带着没有权限文案的提供方去装。
#[tokio::test]
async fn a_failing_provider_preview_keeps_the_real_failure_reason() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/preview",
        Canned(200, serde_json::to_string(&consumer_preview()).unwrap()),
    );
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/preview",
        core_error(502, "upstream", "商店源暂时连不上"),
    );

    let error = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap_err();
    assert!(
        matches!(error, wonton_client::CoreError::Upstream { ref reason } if reason.contains("商店源暂时连不上")),
        "{error:?}"
    );
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty()
    );
}

/// 同一个提供方补两个能力时各列一节，各自保留高权限标记；装上只发一条 install_together。
#[tokio::test]
async fn one_provider_for_two_capabilities_keeps_each_section() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = consumer_preview();
    let mut plain = requires("indexer");
    plain.providers[0].high_privilege = false;
    consumer.missing_requires = vec![plain, requires("x-acme.runner"), requires("torrent_client")];
    // 三个能力都指向同一个提供方（同一个插件提供多个能力）。
    for missing in &mut consumer.missing_requires {
        missing.providers[0].id = "app.downloader".to_owned();
    }
    answer_previews(&rig, &consumer, &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(shown.install_providers.len(), 3);
    assert!(
        shown.install_providers.iter().all(|section| section
            .selected
            .as_ref()
            .unwrap()
            .provider
            .id
            == "app.downloader")
    );
    assert!(!shown.install_providers[0].providers[0].high_privilege);
    assert!(shown.install_providers[1].providers[0].high_privilege);

    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["install_together"].as_array().unwrap().len(), 1);
}

/// 主人勾选的 `wanting_plugins` 只进外层请求的 `connect_plugins`，随批安装的提供方内层保持空（ADR-0053 第 3 条）。
#[tokio::test]
async fn checked_wanting_plugins_go_into_the_outer_request_only() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = consumer_preview();
    consumer.wanting_plugins = vec![wonton_protocol::CapabilityProviderRef {
        id: "app.music".to_owned(),
        title: "音乐".to_owned(),
        high_privilege: true,
    }];
    answer_previews(&rig, &consumer, &provider_preview());

    let mut shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    assert!(shown.connected_plugins.is_empty(), "默认一个都不接");
    shown.connected_plugins.push("app.music".to_owned());
    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["connect_plugins"], serde_json::json!(["app.music"]));
    assert_eq!(
        sent["install_together"][0]["request"]["connect_plugins"],
        serde_json::json!([])
    );
}
