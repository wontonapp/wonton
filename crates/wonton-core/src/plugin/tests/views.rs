//! 管理接口给 App 的视图：已安装列表、详情、安装预览和图标。

use wonton_protocol::{PluginKind, PluginState, PluginTrust};

use super::super::fake_docker::FakeDocker;
use super::super::gallery::Variant;
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::super::package::PluginPackage;
use super::super::package_files::{PackageFiles, sha256_hex};
use super::manager::manager_with;
use super::package;

pub(super) const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="#22aa66"/></svg>"##;

fn photos(icon: &str) -> PluginPackage {
    let yaml = package(
        "  web:\n    image: nginx:1.27",
        "  icon: icon.svg\n  tags: {zh: [相册, AI], en: [Photos, AI]}\n  author: {name: Example}\n  \
         release_notes: {zh: 第一版, en: First release}\n  requirements: {memory: 2G}\n  permissions: []",
    )
    .replace("id: app.test", "id: app.photos")
    .replace("{zh: 测试, en: Test}", "{zh: 相册, en: Photos}");
    let files = PackageFiles::from([("icon.svg".to_owned(), icon.as_bytes().to_vec())]);
    PluginPackage::with_files(&yaml, files).unwrap()
}

fn english() -> Languages {
    Languages::from_header(Some("en-US"))
}

#[tokio::test]
async fn installed_plugins_carry_store_info_and_serve_their_icons() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install_package(photos(ICON), &[]).await.unwrap();

    let list = m.installed(&english());
    let [plugin] = &list.plugins[..] else {
        panic!("{list:?}")
    };
    assert_eq!(plugin.title, "Photos");
    assert_eq!(plugin.kind, PluginKind::App);
    assert_eq!(plugin.trust, PluginTrust::Local);
    assert!(plugin.removable);
    assert!(
        matches!(plugin.status, PluginState::Running { .. }),
        "{:?}",
        plugin.status
    );
    let chinese = Languages::from_header(Some("zh"));
    assert_eq!(m.installed(&chinese).plugins[0].title, "相册");
    assert_eq!(
        m.installed(&Languages::default()).plugins[0].title,
        "Photos"
    );
    let id = PluginId::parse(&plugin.id).unwrap();
    assert_eq!(
        m.installed_detail(&id, &chinese).unwrap().tags,
        ["相册", "AI"]
    );
    assert_eq!(
        m.installed_detail(&id, &english()).unwrap().tags,
        ["Photos", "AI"]
    );

    let icon = plugin.icon.clone().expect("有图标");
    let sha = sha256_hex(ICON.as_bytes());
    assert_eq!(
        icon.small.path,
        format!("/_wonton/api/images/{sha}/icon-192.png")
    );
    let png = m.icon_png(&sha, Variant::Small).await.unwrap().unwrap();
    assert_eq!(image::load_from_memory(&png).unwrap().width(), 192);
    assert_eq!(m.icon_png(&"0".repeat(64), Variant::Large).await, Ok(None));

    let detail = m
        .judged_detail(&PluginId::parse("app.photos").unwrap(), &english())
        .await
        .unwrap();
    assert_eq!(detail.release_notes.as_deref(), Some("First release"));
    assert_eq!(detail.requirements[0].text, "需要 2G 内存");
    assert_eq!(detail.author.unwrap().name, "Example");
}

#[tokio::test]
async fn previews_explain_what_will_be_installed_or_why_not() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let preview = m.preview_install(&photos(ICON), &english()).await.unwrap();
    assert_eq!(preview.digest, photos(ICON).digest());
    assert_eq!(preview.trust, PluginTrust::Local);
    assert_eq!(preview.blocked, None);
    let notices: Vec<&str> = preview.notices.iter().map(|n| n.text.as_str()).collect();
    assert!(
        notices.contains(&"镜像没有按摘要固定：web（nginx:1.27）"),
        "{notices:?}"
    );
    assert!(notices.contains(&"需要 2G 内存"), "{notices:?}");
    // 确认页显示的图标在装之前就取得到。
    let sha = sha256_hex(ICON.as_bytes());
    assert!(m.icon_png(&sha, Variant::Large).await.unwrap().is_some());

    m.install_package(photos(ICON), &[]).await.unwrap();
    let again = m.preview_install(&photos(ICON), &english()).await.unwrap();
    assert!(again.blocked.unwrap().contains("已经装了"));

    let host = package("  web:\n    image: nginx", "  permissions: [network.host]");
    let host = PluginPackage::parse(&host).unwrap();
    let preview = m.preview_install(&host, &english()).await.unwrap();
    assert_eq!(
        preview.blocked.as_deref(),
        Some("核心还不支持权限 network.host")
    );
    assert_eq!(
        preview.permissions[0].risk,
        Some(wonton_protocol::PermissionRisk::VeryHigh)
    );
    assert!(!preview.permissions[0].supported);
}

#[tokio::test]
async fn invalid_icons_fail_the_preview_and_the_install() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let wide = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 24"/>"#;
    let err = m
        .preview_install(&photos(wide), &english())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input");
    assert!(err.to_string().contains("图标 icon.svg"), "{err}");
    let err = m.install_package(photos(wide), &[]).await.unwrap_err();
    assert_eq!(err.kind(), "invalid_input");
    assert!(m.installed(&english()).plugins.is_empty());
}
