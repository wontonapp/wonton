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
async fn the_confirm_page_names_the_provider_that_will_be_installed_with_it() {
    let rig = Rig::new().await;
    rig.connected().await;
    answer_previews(&rig, &consumer_preview(), &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    let line = shown
        .notices
        .iter()
        .find(|line| line.text.contains("下载器"))
        .expect("确认页要写明会一起装提供方");
    assert!(line.emphasis, "高权限提供方要醒目");
    assert!(line.text.contains("torrent_client"), "{line:?}");
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

/// 一起装时确认页没有提供方自己的权限，最高级的“单独确认”（ADR-0036）没处走：这种提供方整单停下。
#[tokio::test]
async fn a_provider_with_risky_permissions_is_not_installed_together() {
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
    install(&rig, shown);
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
            } if message.contains("下载器")
        ),
        "{state:?}"
    );
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty(),
        "不能带着没确认的提供方权限发安装请求"
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
    assert_eq!(
        shown
            .notices
            .iter()
            .filter(|line| line.text.contains("下载器"))
            .count(),
        1,
        "同一个提供方只写一行：{:?}",
        shown.notices
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
    let line = shown
        .notices
        .iter()
        .find(|line| line.text.contains("下载器"))
        .expect("上传包的确认页也要写明会一起装提供方");
    assert!(line.text.contains("torrent_client"), "{line:?}");

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

/// 提供方自己 wants 一个已装的高权限能力：预览里醒目的连接提示（ADR-0053）一起装时会丢掉，
/// 整单停下，请主人单独装它。
#[tokio::test]
async fn a_provider_with_a_prominent_connection_notice_is_not_installed_together() {
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
    install(&rig, shown);
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
            } if message.contains("下载器")
        ),
        "{state:?}"
    );
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty(),
        "不能丢掉醒目的连接提示就把提供方连上"
    );
}

/// 提供方的预览取不到时按真实原因收场：商店源连不上是 Upstream，不归成“输入非法”。
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

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    install(&rig, shown);
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
            } if message.contains("商店源暂时连不上")
        ),
        "{state:?}"
    );
}

/// 同一提供方补的几个能力里高权限的排在后面：这一行仍要醒目（emphasis 取各行 high_privilege 的“或”），
/// 能力名合并在同一行里。
#[tokio::test]
async fn the_provider_notice_stays_prominent_when_the_high_privilege_capability_comes_second() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = consumer_preview();
    let mut plain = requires("indexer");
    plain.providers[0].high_privilege = false;
    consumer.missing_requires = vec![plain, requires("agent_acp")];
    answer_previews(&rig, &consumer, &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    let noticed = shown
        .notices
        .iter()
        .filter(|line| line.text.contains("下载器"))
        .count();
    assert_eq!(noticed, 1, "同一个提供方只写一行：{:?}", shown.notices);
    let line = shown
        .notices
        .iter()
        .find(|line| line.text.contains("下载器"))
        .expect("确认页要写明会一起装提供方");
    assert!(line.emphasis, "第二个能力是高权限时这一行仍要醒目");
    assert!(line.text.contains("indexer"), "{line:?}");
    assert!(line.text.contains("agent_acp"), "{line:?}");
}
