//! 安装这条线按请求的语言写（ADR-0043）：同一场景用 `ja` 和 `fr-FR` 请求，分别拿到 ja 和 en 的
//! 那条；期望值由目录本身格式化，再断言两者不同，防止协商被绕过。

use super::super::PluginId;
use super::super::languages::Languages;
use super::super::manager::PluginStatus;
use super::manager::manager_with;
use crate::l10n::CATALOG;

fn langs(header: &str) -> Languages {
    Languages::from_header(Some(header))
}

fn expected(tag: &str, id: &str, message: &str) -> String {
    let lang = CATALOG.negotiate(&[tag]);
    CATALOG.format(lang, message, &[("id", id.into())])
}

/// 同一个应用装两遍：第二次的冲突原因按请求的语言写。
#[tokio::test]
async fn install_conflict_follows_the_requested_language() {
    let (_docker, engine) = fake_docker().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = super::package(
        "  web:\n    image: nginx\n    volumes: ['${WONTON_APP_DATA}/data:/data']",
        "  permissions: []",
    );
    m.install(&compose, &[], langs("ja")).await.unwrap();

    let id = PluginId::parse("app.test").unwrap();
    for (header, tag) in [("ja", "ja"), ("fr-FR", "en"), ("zh-TW", "zh-Hant")] {
        let error = m.install(&compose, &[], langs(header)).await.unwrap_err();
        assert_eq!(error.kind(), "conflict", "{header}: {error}");
        assert_eq!(
            error.to_string(),
            expected(
                tag,
                id.as_str(),
                "plugin-install-conflict-already-installed"
            ),
            "{header}"
        );
    }
    let ja = m
        .install(&compose, &[], langs("ja"))
        .await
        .unwrap_err()
        .to_string();
    let en = m
        .install(&compose, &[], langs("fr-FR"))
        .await
        .unwrap_err()
        .to_string();
    assert_ne!(ja, en);
}

/// `busy` 对正在安装/卸载/升级的应用按语言回答“正在进行”。
#[test]
fn busy_conflict_follows_the_requested_language() {
    let id = PluginId::parse("app.test").unwrap();
    for (status, message) in [
        (PluginStatus::Installing, "plugin-install-busy-installing"),
        (
            PluginStatus::Uninstalling,
            "plugin-install-busy-uninstalling",
        ),
        (PluginStatus::Upgrading, "plugin-install-busy-upgrading"),
    ] {
        let ja = super::super::manager::busy::busy(&id, &status, &langs("ja"))
            .unwrap()
            .to_string();
        let fr = super::super::manager::busy::busy(&id, &status, &langs("fr-FR"))
            .unwrap()
            .to_string();
        let lang = CATALOG.negotiate(&["ja"]);
        assert_eq!(
            ja,
            CATALOG.format(lang, message, &[("id", id.as_str().into())])
        );
        assert_ne!(ja, fr);
    }
}

async fn fake_docker() -> (
    super::super::fake_docker::FakeDocker,
    super::super::docker::Engine,
) {
    super::super::fake_docker::FakeDocker::start().await
}
