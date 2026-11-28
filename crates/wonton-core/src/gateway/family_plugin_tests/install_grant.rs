//! 装插件的门槛：没有“装、删、升级应用”的家人进不了装卸升级接口和商店。

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, Harness, WHOAMI};
use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;
use crate::plugin::storefront::index;

use super::invite_record;

/// 家人装插件的门槛：没“装、删、升级应用”回 403 permission_required，给了（管理员模板）能走到和主人一样的路。
#[tokio::test]
async fn installing_plugins_needs_the_owners_grant() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    // 装上 echo（FakeDocker 建容器、写记录）：授权前的 403 门在 install 之前。
    let compose = WHOAMI
        .replace("app.whoami", "app.echo")
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    h.manager
        .install(&compose, &[], Languages::from_header(None))
        .await
        .unwrap();

    // 没有“装、删、升级应用”的家人调装卸升级类：403，缺的就是这一项。
    for (method, path, body) in [
        (
            Method::POST,
            "/_wonton/api/plugins/preview",
            "{\"source\": \"github:o/r\"}",
        ),
        (Method::POST, "/_wonton/api/plugins/app.echo/start", ""),
        (Method::DELETE, "/_wonton/api/plugins/app.echo", ""),
        (
            Method::POST,
            "/_wonton/api/plugins/app.echo/upgrade/preview",
            "",
        ),
    ] {
        let reply = send_as(&h.app, Some(hers), method.clone(), CORE, path, &[], body).await;
        assert_eq!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            reply.text()
        );
        assert_eq!(
            reply.error_kind(),
            Some("permission_required"),
            "{method} {path}: {}",
            reply.text()
        );
        assert_eq!(reply.json()["missing"], "install_apps");
    }

    // 主人授权后能走管理接口，也看得到全部插件。
    // 换成有“装、删、升级应用”的角色（管理员模板）。
    h.members.set_role("mia", "admin").unwrap();

    // 能装插件的家人看得到全部插件；启动放行。
    assert_listed_after_grant(&h, hers).await;
}

/// 授权后：列表看得到装着的插件，启动放行。
async fn assert_listed_after_grant(h: &Harness, hers: iroh::EndpointId) {
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[],
        "",
    )
    .await;
    assert!(
        reply.json()["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "app.echo"),
        "{}",
        reply.text()
    );
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::POST,
        CORE,
        "/_wonton/api/plugins/app.echo/start",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
}

/// 授权了“能装插件”的家人看商店和主人一样；没授权的 403（路由表）。
#[tokio::test]
async fn a_family_member_with_install_grant_reads_the_store() {
    // 本地索引：不用外网就有商店内容。
    let index = tempfile::tempdir().unwrap();
    let compose = WHOAMI
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    let package = PluginPackage::parse(&compose, &Languages::from_header(None)).unwrap();
    index::IndexEntry::of(&package, "github:example/whoami", &"a".repeat(40))
        .unwrap()
        .write_to(index.path(), &package)
        .unwrap();
    let h = Harness::start_with_store(
        None,
        crate::plugin::storefront::StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    let hers = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    invite_record(&h, false).await;

    // 没授权：403 permission_required（路由表里也过，这里看有索引时的行为）。
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/store",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("permission_required"));
    assert_eq!(reply.json()["missing"], "install_apps");

    // 主人授权后：商店读得到，官方段里有索引里的插件。
    h.members.set_role("mia", "admin").unwrap();
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/store",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(
        !reply.json()["official"]["plugins"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{}",
        reply.text()
    );
}
