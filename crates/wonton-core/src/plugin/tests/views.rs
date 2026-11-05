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
