//! 清单的 `setup`：装之前就要主人看到的一步，安装和详情页都醒目、排在最前，缺语言的写法不强制。

use super::super::PluginPackage;
use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::manager::manager_with;
use super::package;

fn zh() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

fn with_setup(setup: &str) -> String {
    package(
        "  web:\n    image: nginx",
        &format!("  permissions: []\n  setup: {setup}"),
    )
}

#[tokio::test]
async fn the_install_preview_shows_the_setup_step_first_and_emphasized() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = with_setup(
        "{zh: 把公钥加到宿主的 authorized_keys, en: Add the public key to the host's authorized_keys}",
    );

    let preview = m
        .preview_install(&PluginPackage::parse(&compose, &zh()).unwrap(), &zh())
        .await
        .unwrap();

    assert_eq!(preview.notices[0].text, "把公钥加到宿主的 authorized_keys");
    assert!(preview.notices[0].emphasis, "{:?}", preview.notices);
}

#[tokio::test]
async fn the_detail_page_shows_the_setup_step_too() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = with_setup(
        "{zh: 把公钥加到宿主的 authorized_keys, en: Add the public key to the host's authorized_keys}",
    );
    m.install(&compose, &[], zh()).await.unwrap();

    let detail = m
        .installed_detail(&PluginId::parse("app.test").unwrap(), &zh())
        .unwrap();
    assert_eq!(detail.notices[0].text, "把公钥加到宿主的 authorized_keys");
    assert!(detail.notices[0].emphasis, "{:?}", detail.notices);
}

#[test]
fn setup_must_not_be_blank() {
    let compose = with_setup("{zh: '  ', en: x}");
    let error = PluginPackage::parse(&compose, &zh()).unwrap_err();
    assert!(error.to_string().contains("setup"), "{error}");
}

#[test]
fn setup_has_a_length_limit() {
    let long = "很".repeat(401);
    let compose = with_setup(&format!("{{zh: {long}, en: {long}}}"));
    let error = PluginPackage::parse(&compose, &zh()).unwrap_err();
    assert!(error.to_string().contains("setup"), "{error}");
}
