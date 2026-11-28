//! 插件配置文件的落盘：进容器前写入，只读根的服务从核心目录只读挂载。

use std::io::Read;

use super::super::super::fake_docker::FakeDocker;
use super::super::super::names::container_name;
use super::super::super::package_files::PackageFiles;
use super::super::super::store::Store;
use super::super::super::{PluginId, PluginPackage, PluginStatus};
use super::super::{langs, package};
use super::manager_with;

/// 归档里的文件：（路径，uid，gid，权限，内容）。
fn archived(archive: &[u8]) -> Vec<(String, u64, u64, u32, Vec<u8>)> {
    let mut archive = tar::Archive::new(archive);
    let entries = archive.entries().unwrap();
    entries
        .map(|entry| {
            let mut entry = entry.unwrap();
            let header = entry.header().clone();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            let path = entry.path().unwrap().display().to_string();
            let (uid, gid) = (header.uid().unwrap(), header.gid().unwrap());
            (path, uid, gid, header.mode().unwrap(), data)
        })
        .collect()
}

#[tokio::test]
async fn configs_are_written_after_create_and_before_start() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);
    let compose = format!(
        "configs:\n  app:\n    file: app.yaml\n{}",
        package(
            "  web:\n    image: nginx\n    configs:\n      - source: app\n        \
             target: /etc/app.yaml\n        uid: 101\n        gid: '102'",
            "  permissions: []",
        )
    );
    let files: PackageFiles = [("app.yaml".to_owned(), b"level: info\n".to_vec())].into();
    let package = PluginPackage::with_files(&compose, files.clone(), &langs()).unwrap();
    m.install_package(package, &[], langs()).await.unwrap();

    let web = container_name("app-test", "web");
    let writes = docker.writes();
    let position = |call: String| {
        let found = writes.iter().position(|w| *w == call);
        found.unwrap_or_else(|| panic!("缺少 {call}：{writes:?}"))
    };
    assert!(
        position(format!("PUT /containers/{web}/archive"))
            < position(format!("POST /containers/{web}/start")),
        "配置要在启动前写进容器：{writes:?}"
    );
    // 目标上级目录在容器里不存在时先建出来（root、0755），文件照声明写。
    assert_eq!(
        archived(&docker.archive(&web).unwrap()),
        [
            ("etc".to_owned(), 0, 0, 0o755, Vec::new()),
            (
                "etc/app.yaml".to_owned(),
                101,
                102,
                0o644,
                b"level: info\n".to_vec()
            ),
        ]
    );

    // 插件包的文件随记录保存，核心重启后用它们恢复。
    let records = Store::open(dir.path().join("plugins")).unwrap();
    let record = records.load_all().unwrap().remove(0).record.unwrap();
    assert_eq!(record.files, files);
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
}

/// 只读根的服务：配置文件经一次性容器写进核心数据目录（daemon 按归档落属主和权限），容器以只读
/// 绑定挂载读它们；不再往容器根写归档。卸载时 store 目录跟着删。
#[tokio::test]
async fn a_read_only_service_gets_configs_mounted_from_the_core_store() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);
    let compose = format!(
        "configs:\n  app:\n    file: app.yaml\n{}",
        package(
            "  web:\n    image: nginx\n    read_only: true\n    configs:\n      - source: app\n        \
             target: /etc/app.yaml\n        uid: 65532\n        gid: 65532\n        mode: 0440",
            "  permissions: []",
        )
    );
    let files: PackageFiles = [("app.yaml".to_owned(), b"level: info\n".to_vec())].into();
    let package = PluginPackage::with_files(&compose, files, &langs()).unwrap();
    m.install_package(package, &[], langs()).await.unwrap();

    let web = container_name("app-test", "web");
    let helper = format!("{web}.cfgseed");
    let store = dir.path().join("core/configs/app-test/web");
    assert!(store.is_dir(), "store 目录要建好：{:?}", docker.writes());
    // 归档按编号命名，属主和权限记录在案，交给 daemon 落盘。
    assert_eq!(
        archived(&docker.archive(&helper).unwrap()),
        [(
            "0".to_owned(),
            65532,
            65532,
            0o440,
            b"level: info\n".to_vec()
        )]
    );
    let writes = docker.writes();
    let position = |call: String| {
        writes
            .iter()
            .position(|w| *w == call)
            .unwrap_or_else(|| panic!("缺少 {call}：{writes:?}"))
    };
    assert!(
        position(format!("PUT /containers/{helper}/archive"))
            < position(format!("DELETE /containers/{helper}")),
        "一次性容器写完就删：{writes:?}"
    );
    assert!(
        !docker.containers().iter().any(|c| c == &helper),
        "一次性容器不留下：{:?}",
        docker.containers()
    );
    // 容器只读挂载 store 文件，容器根没有归档。
    let created = docker.created(&web).unwrap();
    let mounts = created["HostConfig"]["Mounts"].as_array().unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0]["Type"], "bind");
    assert_eq!(mounts[0]["Source"], store.join("0").display().to_string());
    assert_eq!(mounts[0]["Target"], "/etc/app.yaml");
    assert_eq!(mounts[0]["ReadOnly"], true);
    assert_eq!(docker.archive(&web), None);

    m.uninstall(&PluginId::parse("app.test").unwrap(), false, &langs())
        .await
        .unwrap();
    assert!(!store.exists(), "卸载要删掉 store 目录");
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    assert!(restarted.list().is_empty());
}
