//! 管理接口给 App 的视图：已安装列表、详情、安装预览和图标。
use std::sync::Arc;

use axum::{Router, routing::get};
use tokio::net::TcpListener;
use wonton_protocol::{PluginKind, PluginState, PluginTrust, TabLayout};

use super::super::egress::address::ProxyAddress;
use super::super::egress::settings::Egress;
use super::super::fake_docker::FakeDocker;
use super::super::gallery::Variant;
use super::super::host_relay::{HostRelay, Purpose};
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::super::package::PluginPackage;
use super::super::package_files::{PackageFiles, sha256_hex};
use super::manager::{manager_with, manager_with_relay_dir};
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
    PluginPackage::with_files(&yaml, files, &super::langs()).unwrap()
}

fn english() -> Languages {
    Languages::from_header(Some("en-US"))
}

#[tokio::test]
async fn installed_plugins_carry_store_info_and_serve_their_icons() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install_package(photos(ICON), &[], super::langs())
        .await
        .unwrap();

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
    assert_eq!(
        detail.requirements[0].text,
        crate::l10n::text(
            &english(),
            "plugin-install-req-memory-line",
            &[("memory", "2G".into())]
        )
    );
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
    m.install_package(original, &[], super::langs())
        .await
        .unwrap();
    let before = m.installed_plugin(&id, &english()).unwrap();
    assert_eq!(before.revision, original_digest);

    let updated = photos(&ICON.replace("#22aa66", "#aa2266"));
    let updated_digest = updated.digest();
    m.upgrade_upload(
        &id,
        updated,
        Vec::new(),
        Vec::new(),
        false,
        &Default::default(),
        &super::langs(),
    )
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
async fn open_urls_carry_the_declared_entry_path_and_query() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/", get(|| async { "ok" }))
                .route("/vnc.html", get(|| async { "ok" })),
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
            "  icon: icon.svg\n  webui: {{service: web, port: {port}, path: '/vnc.html?autoconnect=1&resize=remote'}}\n  permissions: []"
        ),
    )
    .replace("id: app.test", "id: app.entry");
    let package = PluginPackage::with_files(
        &yaml,
        PackageFiles::from([("icon.svg".to_owned(), ICON.as_bytes().to_vec())]),
        &super::langs(),
    )
    .unwrap();
    manager
        .install_package(package, &[], super::langs())
        .await
        .unwrap();

    // 已安装列表（App 主屏）和 PluginView（安装、启动接口和状态页）给的是同一个地址：
    // origin 加清单声明的入口路径，query 原样保留。
    let expected = "http://app-entry.wonton.localhost:8080/vnc.html?autoconnect=1&resize=remote";
    let installed = manager.installed(&english());
    assert_eq!(installed.plugins[0].web_url.as_deref(), Some(expected));
    let views = manager.list();
    let [view] = views.as_slice() else {
        panic!("应有一个插件")
    };
    assert_eq!(view.url.as_deref(), Some(expected));
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
            "  webui: {{service: web, port: {port}, path: /home, cross_origin_isolated: true}}\n  tabs:\n    - id: feed\n      title: {{zh: 动态, en: Feed}}\n      icon: tabs/feed.svg\n      icon_selected: tabs/feed-selected.svg\n      path: /feed\n      layout: edge-to-edge\n  permissions: []"
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
        &super::langs(),
    )
    .unwrap();
    manager
        .install_package(tabs_package, &[], super::langs())
        .await
        .unwrap();

    let root_yaml = package(
        "  web:\n    image: nginx:1.27",
        &format!("  icon: icon.svg\n  webui: {{service: web, port: {port}}}\n  permissions: []"),
    )
    .replace("id: app.test", "id: app.root");
    let root = PluginPackage::with_files(
        &root_yaml,
        PackageFiles::from([("icon.svg".to_owned(), ICON.as_bytes().to_vec())]),
        &super::langs(),
    )
    .unwrap();
    manager
        .install_package(root, &[], super::langs())
        .await
        .unwrap();

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
    // 期望值用目录本身格式化出来（en-US 回落到 en）。
    let unpinned = crate::l10n::text(
        &english(),
        "install-images-unpinned",
        &[(
            "images",
            crate::l10n::text(
                &english(),
                "install-image",
                &[("service", "web".into()), ("image", "nginx:1.27".into())],
            )
            .into(),
        )],
    );
    assert!(notices.contains(&unpinned.as_str()), "{notices:?}");
    assert!(
        notices.contains(
            &crate::l10n::text(
                &english(),
                "plugin-install-req-memory-line",
                &[("memory", "2G".into())]
            )
            .as_str()
        ),
        "{notices:?}"
    );
    // 确认页显示的图标在装之前就取得到。
    let sha = sha256_hex(ICON.as_bytes());
    assert!(m.icon_png(&sha, Variant::Large).await.unwrap().is_some());

    m.install_package(photos(ICON), &[], super::langs())
        .await
        .unwrap();
    let again = m.preview_install(&photos(ICON), &english()).await.unwrap();
    // 期望值用目录本身格式化出来；装的就是同一种包，版本一样。
    let installed = photos(ICON);
    let expected = crate::l10n::text(
        &english(),
        "install-already-installed",
        &[
            ("id", installed.manifest.id.to_string().into()),
            ("version", installed.manifest.version.as_str().into()),
        ],
    );
    assert_eq!(again.blocked.unwrap(), expected);

    let host = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n",
        "  permissions: [network.host]",
    );
    let host = PluginPackage::parse(&host, &super::langs()).unwrap();
    let preview = m.preview_install(&host, &english()).await.unwrap();
    assert_eq!(preview.blocked, None);
    assert_eq!(
        preview.permissions[0].risk,
        Some(wonton_protocol::PermissionRisk::VeryHigh)
    );
    assert!(preview.permissions[0].supported);
}

/// 配了代理时宿主网络的服务也经代理：确认页要如实说，不能还说走宿主自己的路由。
#[tokio::test]
async fn the_preview_says_whether_host_network_services_use_the_proxy() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let compose = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n",
        "  webui: {service: web, port: 8123, sso: none}\n  permissions: [network.host, network.egress]",
    );
    let host = PluginPackage::parse(&compose, &super::langs()).unwrap();
    let notice = |key: &str| crate::l10n::text(&english(), key, &[]);

    let direct = manager_with(engine.clone(), &dir);
    let preview = direct.preview_install(&host, &english()).await.unwrap();
    assert!(
        preview
            .notices
            .iter()
            .any(|n| n.text == notice("install-host-network-egress")),
        "{:?}",
        preview.notices
    );

    let proxy = Egress::new(
        Some(ProxyAddress::parse("http://proxy.lan:1", &english()).unwrap()),
        None,
    );
    let proxied = manager_with(engine, &dir).with_egress(proxy);
    let preview = proxied.preview_install(&host, &english()).await.unwrap();
    let texts: Vec<&str> = preview.notices.iter().map(|n| n.text.as_str()).collect();
    assert!(
        texts.contains(&notice("install-host-network-proxied").as_str()),
        "{texts:?}"
    );
    assert!(!texts.contains(&notice("install-host-network-egress").as_str()));
}

#[tokio::test]
async fn a_too_long_relay_socket_path_is_blocked_in_the_preview() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    // 套接字只在 `/run` 下按哈希命名（16 位加 `.sock`，末段 21 字节）；中继目录被指到长路径时仍然超上限，
    // 预检要报出来。join 的段之间还有一个分隔符字节。
    let base = dir.path().as_os_str().len();
    let relay_dir = dir.path().join("d".repeat(120 - base - 1));
    assert_eq!(
        relay_dir.join("0000000000000000.sock").as_os_str().len(),
        142
    );
    let m = manager_with_relay_dir(engine, &dir, &relay_dir);
    let host = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n",
        "  webui: {service: web, port: 8123, sso: none}\n  permissions: [network.host]",
    );
    let host = PluginPackage::parse(&host, &super::langs()).unwrap();
    let preview = m.preview_install(&host, &english()).await.unwrap();
    let blocked = preview.blocked.unwrap();
    let socket = super::super::host_relay::socket_path(
        &relay_dir,
        &Default::default(),
        "app-test",
        &HostRelay {
            purpose: Purpose::Webui,
            service: "web".to_owned(),
            port: 8123,
        },
    );
    let expected = crate::l10n::text(
        &english(),
        "plugin-install-relay-path-too-long",
        &[
            ("path", socket.display().to_string().into()),
            ("bytes", socket.as_os_str().len().into()),
            ("max", 107usize.into()),
        ],
    );
    assert_eq!(blocked, expected);
    // 中继镜像是核心自己的，已按摘要固定，不再提示未固定。
    let notices: Vec<&str> = preview.notices.iter().map(|n| n.text.as_str()).collect();
    assert!(
        !notices.iter().any(|n| n.contains("relay-webui")),
        "{notices:?}"
    );
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
    let host = PluginPackage::parse(&host, &super::langs()).unwrap();
    let zh = m
        .preview_install(&host, &Languages::from_header(Some("zh")))
        .await
        .unwrap();
    let zh_notice = zh
        .permissions
        .iter()
        .find(|line| line.permission == "network.host")
        .and_then(|line| line.notice.as_deref());
    // 期望值用目录本身格式化出来。
    assert_eq!(
        zh_notice,
        Some(
            crate::l10n::text(
                &Languages::from_header(Some("zh")),
                "permission-notice-network-host",
                &[]
            )
            .as_str()
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
            crate::l10n::text(
                &Languages::from_header(Some("en-US")),
                "permission-notice-network-host",
                &[]
            )
            .as_str()
        )
    );

    let socket = package(
        "  web:\n    image: nginx\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
        "  permissions: [docker.socket]",
    );
    let socket = PluginPackage::parse(&socket, &super::langs()).unwrap();
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
            crate::l10n::text(
                &Languages::from_header(Some("en-US")),
                "permission-notice-highest-risk",
                &[]
            )
            .as_str()
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
    // 期望值用目录本身格式化出来；reason 是图片 worker 的原话（英文），只核对目录里的固定前缀。
    let prefix = crate::l10n::text(
        &english(),
        "plugin-install-install-icon-error",
        &[("path", "icon.svg".into()), ("reason", "".into())],
    );
    assert!(err.to_string().starts_with(prefix.trim_end()), "{err}");
    let err = m
        .install_package(photos(wide), &[], super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input");
    assert!(m.installed(&english()).plugins.is_empty());
}
