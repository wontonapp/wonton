//! 要不要入口、要不要删：无设置、撤设置、缺文件重建、幂等，核心不在容器里。

use super::super::EntryContainers;
use super::test_support::*;
use crate::entry_haproxy::CONNECTOR_ADDRESS;
use crate::plugin::names::Instance;

/// 没有入口设置：两个容器都不在，也不去动网络。
#[tokio::test]
async fn no_entry_settings_no_containers() {
    let rig = Rig::new().await;
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.others(), Vec::<String>::new());
    assert!(rig.writes_for("/networks/").is_empty());
}

/// 重复对账不重建、不改网络。
#[tokio::test]
async fn reconcile_is_idempotent() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
    assert_eq!(rig.docker.creations("wonton-connector"), 1);
    // 端点都在创建请求里，第二次对账既不重建也不补接网络。
    let connects = rig.writes_for("/connect").len();
    assert_eq!(connects, 0, "{connects}");
}

/// 设置撤了：两个容器都删掉。
#[tokio::test]
async fn removed_settings_remove_containers() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.others(), Vec::<String>::new());
}

/// 全新安装时 `edge` 就是默认网段，算出来的连接器地址和装机时写的一样：配置不能因此不写。
/// 缺文件时 Docker 会在宿主侧把这些路径建成目录，haproxy 在参数解析阶段就退出 1（复审 878 第 1 条）。
/// 对账必须把目录换回文件。
#[tokio::test]
async fn the_default_edge_subnet_still_gets_the_whole_entry_config() {
    let rig = Rig::new().await;
    rig.docker.pin_subnet(
        &format!("{PROJECT}_edge"),
        crate::entry_haproxy::EDGE_SUBNET,
    );
    app_domain(&rig, "home.example.test");
    // 旧版没写文件就建了容器：挂载源已经被 Docker 建成了目录。
    for name in [
        "entry/haproxy.cfg",
        "entry/certs/public.crt-list",
        "entry/certs/lan.crt-list",
        "entry/connector.cfg",
    ] {
        std::fs::create_dir_all(rig.dir.path().join(name)).unwrap();
    }
    rig.reconcile(None).await.unwrap();
    eprintln!(
        "DEBUG writes {:?} containers {:?} others {:?}",
        rig.docker.writes(),
        rig.docker.containers(),
        rig.others()
    );
    let config = config(&rig);
    assert!(!config.is_empty(), "默认网段下入口配置没写出来");
    assert!(
        config.contains(&format!("src {CONNECTOR_ADDRESS}")),
        "{config}"
    );
    assert!(
        std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list"))
            .unwrap()
            .contains("home.example.test")
    );
    assert!(rig.dir.path().join("entry/certs/lan.crt-list").is_file());
    let connector_config = command_file(&rig, "wonton-connector");
    assert!(
        connector_config.is_file(),
        "{} 不在",
        connector_config.display()
    );
    for container in ["wonton-entry", "wonton-connector"] {
        assert_mounts_exist(&rig, container);
    }
}

/// 核心跑在宿主上：要入口就回 `core_not_in_container`，不建容器。
#[tokio::test]
async fn core_outside_container_reports_core_not_in_container() {
    let rig = Rig::new().await;
    let here = EntryContainers::new(
        rig.docker.engine().await,
        rig.dir.path().to_owned(),
        rig.dir.path().join("data"),
        Instance::default(),
        None,
    );
    let error = here.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "core_not_in_container");
    assert_eq!(
        error.to_string(),
        "core_not_in_container：核心不在容器里，没有入口要的 clients、edge 网络"
    );
    assert_eq!(rig.others(), Vec::<String>::new());
    // 没有设置时是空操作：开发栈没有入口设置不该每次启动都报错。
    here.reconcile(None).await.unwrap();
}
