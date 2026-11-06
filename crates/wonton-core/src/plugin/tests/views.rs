//! 管理接口给 App 的视图：已安装列表、详情、安装预览和图标。
use std::sync::Arc;

use axum::{Router, routing::get};
use tokio::net::TcpListener;
use wonton_protocol::{PluginKind, PluginState, PluginTrust, TabDisplay, TabLayout};

use super::super::fake_docker::FakeDocker;
use super::super::gallery::Variant;
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::super::package::PluginPackage;
use super::super::package_files::{PackageFiles, sha256_hex};
use super::manager::manager_with;
use super::package;

pub(super) const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="#22aa66"/></svg>"##;
pub(super) const TAB_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" fill="#000000"/></svg>"##;
pub(super) const TAB_ICON_SELECTED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" fill="#000000"/></svg>"##;

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
async fn content_revision_changes_without_a_version_bump_and_survives_restore() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine.clone(), &dir));
    let id = PluginId::parse("app.photos").unwrap();
    let original = photos(ICON);
    let original_digest = original.digest();
    m.install_package(original, &[]).await.unwrap();
    let before = m.installed_plugin(&id, &english()).unwrap();
    assert_eq!(before.revision, original_digest);

    let updated = photos(&ICON.replace("#22aa66", "#aa2266"));
    let updated_digest = updated.digest();
    m.upgrade_upload(&id, updated, Vec::new(), false)
        .await
        .unwrap();
    let after = m.installed_plugin(&id, &english()).unwrap();
    assert_eq!(after.version, before.version);
    assert_ne!(after.revision, before.revision);
    assert_eq!(after.revision, updated_digest);
    assert_eq!(m.installed(&english()).plugins[0].revision, after.revision);
    assert_eq!(
        m.installed_detail(&id, &english())
            .unwrap()
            .installed
            .unwrap()
            .revision,
        after.revision
    );

    drop(m);
    let restored = manager_with(engine, &dir);
    restored.restore().await.unwrap();
    let plugin = restored.installed_plugin(&id, &english()).unwrap();
    assert!(matches!(plugin.status, PluginState::Running { .. }));
    assert_eq!(plugin.revision, after.revision);
    assert_eq!(
        restored.installed(&english()).plugins[0].revision,
        after.revision
    );
    assert_eq!(
        restored
            .installed_detail(&id, &english())
            .unwrap()
            .installed
            .unwrap()
            .revision,
        after.revision
    );
}

#[tokio::test]
async fn installed_plugins_carry_tabs_and_choose_the_requested_title_language() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/", get(|| async { "ok" }))
                .route("/home", get(|| async { "ok" })),
        )
        .await
        .unwrap();
    });
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let yaml = package(
        "  web:\n    image: nginx:1.27",
        &format!(
            "  webui: {{service: web, port: {port}, path: /home, display: fullscreen, cross_origin_isolated: true}}\n  tabs:\n    - id: feed\n      title: {{zh: 动态, en: Feed}}\n      icon: tabs/feed.svg\n      icon_selected: tabs/feed-selected.svg\n      path: /feed\n      layout: edge-to-edge\n      display: fullscreen\n  permissions: []"
        ),
    )
    .replace("id: app.test", "id: app.tabs")
    .replace("{zh: 测试, en: Test}", "{zh: 标签, en: Tabs}");
    let tabs_package = PluginPackage::with_files(
        &yaml,
        PackageFiles::from([
            ("tabs/feed.svg".to_owned(), TAB_ICON.as_bytes().to_vec()),
            (
                "tabs/feed-selected.svg".to_owned(),
                TAB_ICON_SELECTED.as_bytes().to_vec(),
            ),
        ]),
    )
    .unwrap();
    manager.install_package(tabs_package, &[]).await.unwrap();

    let root_yaml = package(
        "  web:\n    image: nginx:1.27",
        &format!("  icon: icon.svg\n  webui: {{service: web, port: {port}}}\n  permissions: []"),
    )
    .replace("id: app.test", "id: app.root");
    let root = PluginPackage::with_files(
        &root_yaml,
        PackageFiles::from([("icon.svg".to_owned(), ICON.as_bytes().to_vec())]),
    )
    .unwrap();
    manager.install_package(root, &[]).await.unwrap();

    let english = manager.installed(&Languages::from_header(Some("en-US")));
    let plugin = english
        .plugins
        .iter()
        .find(|plugin| plugin.id == "app.tabs")
        .unwrap_or_else(|| panic!("{english:?}"));
    let [tab] = &plugin.tabs[..] else {
        panic!("{plugin:?}")
    };
    assert_eq!(tab.title, "Feed");
    assert_eq!(tab.path, "/feed");
    assert_eq!(tab.layout, TabLayout::EdgeToEdge);
    assert_eq!(tab.display, TabDisplay::Fullscreen);
    assert_eq!(plugin.web_display, TabDisplay::Fullscreen);
    assert!(plugin.cross_origin_isolated);
    assert!(
        plugin
            .web_url
            .as_ref()
            .is_some_and(|url| url.ends_with("/home"))
    );
    let root = english
        .plugins
        .iter()
        .find(|plugin| plugin.id == "app.root")
        .unwrap();
    assert_eq!(
        root.web_url.as_deref(),
        Some("http://app-root.wonton.localhost:8080/")
    );
    assert!(!root.cross_origin_isolated);

    let chinese = manager.installed(&Languages::from_header(Some("zh")));
    let tabs_plugin = chinese
        .plugins
        .iter()
        .find(|plugin| plugin.id == "app.tabs")
        .unwrap();
    assert_eq!(tabs_plugin.tabs[0].title, "动态");
    let sha = sha256_hex(TAB_ICON.as_bytes());
    assert!(
        manager
            .icon_png(&sha, Variant::TabSmall)
            .await
            .unwrap()
            .is_some()
    );
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

    let host = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n",
        "  permissions: [network.host]",
    );
    let host = PluginPackage::parse(&host).unwrap();
    let preview = m.preview_install(&host, &english()).await.unwrap();
    assert_eq!(preview.blocked, None);
    assert_eq!(
        preview.permissions[0].risk,
        Some(wonton_protocol::PermissionRisk::VeryHigh)
    );
    assert!(preview.permissions[0].supported);
}

#[tokio::test]
async fn privilege_previews_use_adr_notices_in_both_languages() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let host = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n",
        "  permissions: [network.host]",
    );
    let host = PluginPackage::parse(&host).unwrap();
    let zh = m
        .preview_install(&host, &Languages::from_header(Some("zh")))
        .await
        .unwrap();
    let zh_notice = zh
        .permissions
        .iter()
        .find(|line| line.permission == "network.host")
        .and_then(|line| line.notice.as_deref());
    assert_eq!(
        zh_notice,
        Some(
            "这个插件能直接访问你家里的网络和本机所有端口，包括只给本机用的服务，也能绕过 Wonton 的登录连到部分其他插件。它开的端口会出现在本机所有网卡上，包括 Wi-Fi、Tailscale 和 IPv6 地址，本机有公网地址时互联网也能连上。本机开着防火墙时，局域网设备连不进来，要你自己放行。"
        )
    );

    let en = m
        .preview_install(&host, &Languages::from_header(Some("en-US")))
        .await
        .unwrap();
    let en_notice = en
        .permissions
        .iter()
        .find(|line| line.permission == "network.host")
        .and_then(|line| line.notice.as_deref());
    assert_eq!(
        en_notice,
        Some(
            "This plugin can directly access your home network and all ports on this machine, including services bound only to localhost, and bypass Wonton login to connect to some other plugins. Its ports will appear on all network interfaces, including Wi-Fi, Tailscale, and IPv6 addresses; if this machine has a public address, the internet can reach them too. If the machine firewall is on, LAN devices cannot connect until you allow them yourself."
        )
    );

    let socket = package(
        "  web:\n    image: nginx\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
        "  permissions: [docker.socket]",
    );
    let socket = PluginPackage::parse(&socket).unwrap();
    let highest = m
        .preview_install(&socket, &Languages::from_header(Some("en-US")))
        .await
        .unwrap();
    let highest_notice = highest
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .and_then(|line| line.notice.as_deref());
    assert_eq!(
        highest_notice,
        Some(
            "This plugin can control the entire host: read and write all files and other plugins' data, change or stop Wonton itself. This is equivalent to giving it root on the host. Only install plugins you trust. After uninstalling, containers it created and things it changed will not be cleaned up."
        )
    );
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
