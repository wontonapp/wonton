//! 文件服务访问插件内网地址前的核对（ADR-0020 的“鉴权”）：挡 SSRF 就靠这里。

use super::super::bot_services::bot_alias;
use super::super::fake_docker::FakeDocker;
use super::super::package::PackageError;
use super::super::upstreams::ResourceUpstreamError;
use super::super::{PluginId, PluginPackage};
use super::manager::{manager_in_container, manager_with};
use super::package;
use crate::files::test_support::any_langs;
use crate::files::{CoreHost, FileHost, resource::InternalUrl};
use crate::mcp::test_support::{Notes, serve_notes};
use crate::mcp::upstream::Upstreams;
use std::sync::Arc;
use wonton_protocol::FileFailure;

fn app(id: &str, service: &str) -> String {
    package(
        &format!("  {service}:\n    image: nginx"),
        "  permissions: []",
    )
    .replace("id: app.test", &format!("id: {id}"))
}

/// 核心接在每个插件网络上，插件的服务名会成为那里的别名，也拼进 `wonton.internal` 的稳定域名，所以必须是
/// 合法的 DNS 标签。不能和系统服务的别名（带点）、别的插件的容器名（带 `--`）一样。
#[test]
fn a_service_cannot_take_the_name_of_a_system_service_or_another_plugins_container() {
    for name in [
        "tusd.wonton.internal",
        "wonton-app-other--web",
        "wonton-app-other-",
        // 下划线进不了 DNS 名字。
        "my-web_1",
    ] {
        let err = PluginPackage::parse(&app("app.mine", name), &super::langs()).unwrap_err();
        assert!(matches!(err, PackageError::Service { .. }), "{name}: {err}");
    }
    PluginPackage::parse(&app("app.mine", "my-web"), &super::langs()).unwrap();
}

/// Bot 服务多一个别名 `<服务名>-default`：不能和别的插件的容器名 `wonton-app-b--default` 一样。
#[test]
fn a_generated_alias_cannot_take_the_name_of_another_plugins_container() {
    assert_eq!(bot_alias("wonton-app-b-"), "wonton-app-b--default");

    let bot = package(
        "  wonton-app-b-:\n    image: hermes",
        "  bot: {display_name: {zh: 测试, en: Test}}\n  bots: {services: [wonton-app-b-]}\n  \
         permissions: [matrix.bot]",
    );
    let err = PluginPackage::parse(&bot, &super::langs()).unwrap_err();
    assert!(matches!(err, PackageError::Service { .. }), "{err}");
}

#[tokio::test]
async fn only_a_plugins_own_services_have_an_upstream() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&app("app.mine", "server"), &[], super::langs())
        .await
        .unwrap();
    m.install(&app("app.other", "theirs"), &[], super::langs())
        .await
        .unwrap();
    let mine = PluginId::parse("app.mine").unwrap();

    // 这个假的 Docker 给每个容器的地址都是 127.0.0.1。
    let own = m.resource_upstream(&mine, "server", 2283).await.unwrap();
    assert_eq!(own, "127.0.0.1:2283");

    for host in [
        "theirs",
        "hydra",
        "tuwunel",
        "tusd",
        "core",
        "127.0.0.1",
        "localhost",
        "[::1]",
        "169.254.169.254",
        "hydra.wonton.internal",
        "app-other--theirs",
        "",
    ] {
        let refused = m.resource_upstream(&mine, host, 4445).await;
        assert!(
            matches!(refused, Err(ResourceUpstreamError::NotItsService)),
            "{host}: {refused:?}"
        );
    }

    let missing = PluginId::parse("app.missing").unwrap();
    let refused = m.resource_upstream(&missing, "server", 2283).await;
    assert!(matches!(refused, Err(ResourceUpstreamError::Unavailable)));

    m.uninstall(&mine, false, &super::langs()).await.unwrap();
    let refused = m.resource_upstream(&mine, "server", 2283).await;
    assert!(matches!(refused, Err(ResourceUpstreamError::Unavailable)));
}

/// 部署版的核心和插件在同一个网络里：按服务的稳定域名连（上游看到的 Host 就是它），不问 Docker 要 IP。
#[tokio::test]
async fn inside_a_container_the_core_reaches_a_service_by_its_service_host() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir);
    m.install(&app("app.mine", "server"), &[], super::langs())
        .await
        .unwrap();
    let mine = PluginId::parse("app.mine").unwrap();

    let own = m.resource_upstream(&mine, "server", 2283).await.unwrap();
    assert_eq!(own, "server.app-mine.wonton.internal:2283");

    let refused = m.resource_upstream(&mine, "theirs", 2283).await;
    assert!(matches!(refused, Err(ResourceUpstreamError::NotItsService)));
}

/// Bot 每次调文件工具都看来源插件现在的授权：读或写都算，别的共享文件夹、卸了的插件都不算。
#[tokio::test]
async fn share_grants_come_from_what_the_plugin_was_granted() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let package = app("app.mine", "server").replace(
        "permissions: []",
        "permissions: [\"storage.share.photos:read\", \"storage.share.docs:write\"]",
    );
    let granted = crate::plugin::PluginPackage::parse(&package, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    m.install(&package, &granted, super::langs()).await.unwrap();
    let mine = PluginId::parse("app.mine").unwrap();
    let host = CoreHost::new(Arc::new(m), Arc::new(Upstreams::new()), None);

    assert!(host.share_granted(&mine, "photos").unwrap());
    assert!(host.share_granted(&mine, "docs").unwrap());
    assert!(!host.share_granted(&mine, "music").unwrap());
    let gone = PluginId::parse("app.gone").unwrap();
    assert!(!host.share_granted(&gone, "photos").unwrap());
}

/// 插件的记录读不出来：不是插件不在，也不是地址不对，是核心这边出了问题。
#[tokio::test]
async fn an_unreadable_plugin_record_is_the_cores_problem() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&app("app.mine", "server"), &[], super::langs())
        .await
        .unwrap();
    let mine = PluginId::parse("app.mine").unwrap();
    std::fs::write(dir.path().join("plugins/app.mine.json"), "{ 坏了").unwrap();
    let host = CoreHost::new(Arc::new(m), Arc::new(Upstreams::new()), None);

    let url = InternalUrl::parse("http://server:2283/a", &any_langs()).unwrap();
    let failure = host.service_upstream(&mine, &url).await.unwrap_err();
    assert_eq!(failure.kind, FileFailure::Internal);
}

/// 读不到资源的原因走完整条路（真的 MCP 客户端、假插件）：插件说不行、连不上、超过上限各是各的类别。
#[tokio::test]
async fn the_core_host_tells_why_a_resource_cannot_be_read() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let manifest =
        crate::plugin::PluginPackage::parse(&app("app.notes", "server"), &super::langs())
            .unwrap()
            .manifest;
    let notes = manifest.id.clone();
    m.register_for_test(manifest, crate::plugin::PluginStatus::Running, None);
    m.mcp_for_test(&notes, serve_notes(Notes::default()).await);
    let host = CoreHost::new(m.clone(), Arc::new(Upstreams::new()), None);

    let read = async |uri: &'static str| host.read_resource(&notes, uri).await.map_err(|e| e.kind);
    assert_eq!(read("notes://blob").await.unwrap(), b"abc");
    assert_eq!(
        read("notes://missing").await,
        Err(FileFailure::UpstreamError)
    );
    assert_eq!(
        read("notes://huge").await,
        Err(FileFailure::ResourceTooLarge)
    );

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    m.mcp_for_test(&notes, format!("http://{address}/mcp"));
    assert_eq!(
        read("notes://blob").await,
        Err(FileFailure::PluginUnavailable)
    );
}

#[tokio::test]
async fn the_core_host_tells_why_a_service_has_no_upstream() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&app("app.mine", "server"), &[], super::langs())
        .await
        .unwrap();
    let mine = PluginId::parse("app.mine").unwrap();
    let host = CoreHost::new(Arc::new(m), Arc::new(Upstreams::new()), None);

    let kind = async |plugin: &PluginId, url: &str| {
        let url = InternalUrl::parse(url, &any_langs()).unwrap();
        host.service_upstream(plugin, &url)
            .await
            .map_err(|e| e.kind)
    };
    assert_eq!(
        kind(&mine, "http://server:2283/a").await.unwrap(),
        "127.0.0.1:2283"
    );
    assert_eq!(
        kind(&mine, "http://theirs:2283/a").await,
        Err(FileFailure::PermissionDenied)
    );
    let missing = PluginId::parse("app.missing").unwrap();
    assert_eq!(
        kind(&missing, "http://server:2283/a").await,
        Err(FileFailure::PluginUnavailable)
    );

    // 这个插件没有声明 MCP 服务。
    let unreadable = host.read_resource(&mine, "x://a").await.unwrap_err();
    assert_eq!(unreadable.kind, FileFailure::PluginUnavailable);
    assert_eq!(
        host.check_mcp(&mine).unwrap_err().kind,
        FileFailure::PluginUnavailable
    );
}
