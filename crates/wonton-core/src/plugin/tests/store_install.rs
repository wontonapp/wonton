//! 商店来源安装：#678 的 AutoBangumi。
//!
//! 这两个包的 compose 和文件原样来自上游仓库；AutoBangumi 依赖 qBittorrent 当下载器，所以先装它再装
//! AutoBangumi，和主人从商店点安装走的是同一条 `install_source`。

use std::sync::Arc;

use serde_json::json;
use wonton_protocol::{InstallPreview, InstallRequest};

use super::super::PluginPackage;
use super::super::fake_docker::{FakeDocker, FakeFile};
use super::super::names::container_name;
use super::super::storefront::StoreConfig;
use super::super::storefront::index::IndexEntry;
use super::super::{InstallError, PluginId};
use super::capability_deps_install::id;
use super::langs;
use super::manager::manager_with;

const AB: &str = "github:geraldpeng6/wonton-plugin-autobangumi";
const QB: &str = "github:geraldpeng6/wonton-plugin-qbittorrent";
const ORG: &str = "org.autobangumi.autobangumi";
/// 开发栈的商店索引：仓库里 commit 了这两个包，夹具要和它逐字节一致（上游打了 tag 之后跑 update.sh 更新）。
const INDEX: &str = include_str!("../../../../../deploy/dev/core/store/index.json");

fn committed(source: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(INDEX).unwrap()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["source"] == source)
        .cloned()
        .unwrap_or_else(|| panic!("索引里没有 {source}"))
}

fn package(compose: &'static str, dir: &str, files: &[&str]) -> PluginPackage {
    let files = files
        .iter()
        .map(|name| {
            let bytes: &'static [u8] = match (dir, *name) {
                ("ab", "config/seed.sh") => {
                    include_bytes!("examples/autobangumi/config/seed.sh")
                }
                ("ab", "config/mcp-token.sh") => {
                    include_bytes!("examples/autobangumi/config/mcp-token.sh")
                }
                ("ab", "NOTICE") => include_bytes!("examples/autobangumi/NOTICE"),
                ("ab", "icon.svg") => include_bytes!("examples/autobangumi/icon.svg"),
                ("qb", "config/init.sh") => include_bytes!("examples/qbittorrent/config/init.sh"),
                ("qb", "config/qBittorrent.conf") => {
                    include_bytes!("examples/qbittorrent/config/qBittorrent.conf")
                }
                ("qb", "NOTICE") => include_bytes!("examples/qbittorrent/NOTICE"),
                ("qb", "icon.svg") => include_bytes!("examples/qbittorrent/icon.svg"),
                _ => panic!("未知夹具 {dir}/{name}"),
            };
            (name.to_string(), bytes.to_vec())
        })
        .collect();
    PluginPackage::with_files(compose, files, &langs()).unwrap()
}

fn autobangumi() -> PluginPackage {
    package(
        include_str!("examples/autobangumi/docker-compose.yml"),
        "ab",
        &[
            "config/seed.sh",
            "config/mcp-token.sh",
            "NOTICE",
            "icon.svg",
        ],
    )
}

fn qbittorrent() -> PluginPackage {
    package(
        include_str!("examples/qbittorrent/docker-compose.yml"),
        "qb",
        &[
            "config/init.sh",
            "config/qBittorrent.conf",
            "NOTICE",
            "icon.svg",
        ],
    )
}

fn request(preview: InstallPreview) -> InstallRequest {
    InstallRequest {
        source: preview.source.unwrap(),
        digest: preview.digest,
        plan: preview.plan,
        audience: None,
        scope: None,
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    }
}

/// 网页就绪探针打 `127.0.0.1:<port>`：把夹具里的端口换成本次测试现起的端口。
async fn served(mut package: PluginPackage, port: u16) -> PluginPackage {
    let address = super::egress::serve(axum::Router::new().fallback(|| async { "ok" })).await;
    package.manifest.webui.as_mut().unwrap().port = address.port();
    package.source = package
        .source
        .replace(&port.to_string(), &address.port().to_string());
    package
}

/// 造一个本地商店：索引里是这两个包，安装走和 App 一样的商店来源。
async fn store() -> (
    Arc<super::super::PluginManager>,
    FakeDocker,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let index = tempfile::tempdir().unwrap();
    // 夹具和仓库里的索引逐字节一致；网页端口按测试现起的端口改写后再写进临时索引。
    for (source, fixture, package) in [
        (AB, autobangumi(), served(autobangumi(), 7892).await),
        (QB, qbittorrent(), served(qbittorrent(), 8080).await),
    ] {
        let committed = committed(source);
        let entry =
            IndexEntry::of(&fixture, source, committed["commit"].as_str().unwrap()).unwrap();
        assert_eq!(entry.version, committed["version"], "{source}");
        assert_eq!(
            serde_json::to_value(&entry.files).unwrap(),
            committed["files"],
            "{source}"
        );
        let entry =
            IndexEntry::of(&package, source, committed["commit"].as_str().unwrap()).unwrap();
        entry.write_to(index.path(), &package).unwrap();
    }
    let (docker, engine) = FakeDocker::start().await;
    let data = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &data).with_store(StoreConfig {
        index_dir: Some(index.path().to_owned()),
        community: false,
    });
    // 两个临时目录都要活到测试结束：管理器的数据目录是 `data`，释放了就写不了插件记录。
    (Arc::new(manager), docker, index, data)
}

fn exited(container: &str) -> (String, serde_json::Value) {
    (
        container.to_owned(),
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    )
}

fn healthy(container: &str) -> (String, serde_json::Value) {
    (
        container.to_owned(),
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    )
}

/// 一次性服务按真实 Docker 的样子跑完：`seed`、`mcp-token`、`init` 退出码 0；网页服务报健康。
fn finish(docker: &FakeDocker, plugin: &PluginId, one_shots: &[&str], healthy_services: &[&str]) {
    let slug = plugin.slug();
    for service in one_shots {
        let (name, state) = exited(&container_name(&slug, service));
        docker.set_state(&name, state.clone());
        // 起容器时按这个状态落下：真 Docker 里一次性服务起来就跑完，不等测试改。
        docker.set_start_state(&name, state);
    }
    for service in healthy_services {
        let (name, state) = healthy(&container_name(&slug, service));
        docker.set_start_state(&name, state);
    }
}

async fn install(
    manager: &Arc<super::super::PluginManager>,
    docker: &FakeDocker,
    source: &str,
) -> Result<String, InstallError> {
    let began = std::time::Instant::now();
    let preview = manager.preview_source(source, &langs()).await?;
    eprintln!("preview {} in {:?}", preview.id, began.elapsed());
    let plugin = PluginId::parse(&preview.id).unwrap();
    match plugin.slug().as_str() {
        "org-autobangumi-autobangumi" => {
            finish(docker, &plugin, &["seed", "mcp-token"], &["autobangumi"]);
            docker.set_file(
                &container_name(&plugin.slug(), "mcp-token"),
                "/bridge/token",
                Some(FakeFile::Regular(b"tok".to_vec())),
            );
        }
        _ => finish(docker, &plugin, &["init"], &["webui", "qbittorrent"]),
    }
    let result = manager.install_source(&request(preview)).await;
    eprintln!(
        "install {:?} in {:?}: {:?}",
        result,
        began.elapsed(),
        result.as_ref().map(|v| v.status.clone())
    );
    result.map(|view| view.id.as_str().to_owned())
}

#[tokio::test]
async fn autobangumi_from_store_source_installs() {
    let (manager, docker, _index, _data) = store().await;
    install(&manager, &docker, QB).await.unwrap();
    let installed = match install(&manager, &docker, AB).await {
        Ok(installed) => installed,
        Err(error) => {
            let seed = container_name(&id(ORG).slug(), "seed");
            for call in docker.writes() {
                if call.contains(&seed) {
                    eprintln!("seed call: {call}");
                }
            }
            panic!("FAILED kind={} message={}", error.kind(), error)
        }
    };
    // 没人依赖的一次性服务也要起：少了 mcp-token 这条，MCP 令牌就没有人签发。
    for service in ["seed", "mcp-token"] {
        let name = container_name(&id(ORG).slug(), service);
        assert_eq!(docker.creations(&name), 1, "{service}");
    }
    assert_eq!(installed, ORG);
}

#[tokio::test]
async fn autobangumi_from_store_source_without_token_says_why() {
    let (manager, docker, _index, _data) = store().await;
    install(&manager, &docker, QB).await.unwrap();
    let preview = manager.preview_source(AB, &langs()).await.unwrap();
    let plugin = id(ORG);
    finish(&docker, &plugin, &["seed", "mcp-token"], &["autobangumi"]);
    let error = manager
        .install_source(&request(preview))
        .await
        .expect_err("没有令牌应当装不上");
    assert_eq!(error.kind(), "invalid_input");
    let message = error.to_string();
    for expected in ["mcp_token", "mcp-token", "/bridge/token"] {
        assert!(message.contains(expected), "{expected}: {message}");
    }
}
