//! 真实 Docker：`read_only` 服务的 `configs` 交付（#124）。
//!
//! 验收的几条：装得上、容器里读得到、根文件系统只读、属主和权限照声明；内容更新在升级重建容器后
//! 可见；卸载后核心写的文件没了。

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use super::super::docker::Engine;
use super::super::docker_exec::ExecOutput;
use super::super::{PluginId, PluginPackage, PluginStatus};
use super::manager::manager_in_instance;
use super::package;
use crate::test_docker::Names;

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn image() -> String {
    wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
}

/// 一个 `read_only` 服务，配置放在 `/etc/motd`，属主和权限按声明。
fn compose(content: &str) -> String {
    format!(
        "configs:\n  app:\n    content: \"{content}\"\n{}",
        package(
            &format!(
                "  web:\n    image: {}\n    read_only: true\n    user: '65532:65532'\n    \
                 command: ['sleep', 'infinity']\n    configs:\n      - source: app\n        \
                 target: /etc/motd\n        uid: '65532'\n        gid: '65532'\n        mode: 0440",
                image()
            ),
            "  permissions: []",
        )
    )
}

#[tokio::test]
#[ignore = "需要 Docker，真实容器验证只读根的 configs 交付"]
async fn a_read_only_service_reads_configs_mounted_from_the_core_store() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    crate::test_docker::ensure_image(&engine, &image()).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    // 默认的 5 秒熬不过无健康检查服务要求的 3 秒稳定运行（STABLE_FOR）加安装步骤。
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name());
    manager.cfg.install_timeout = Duration::from_secs(60);
    let m = Arc::new(manager);
    m.install(&compose("v1\\n"), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(m.list()[0].status, PluginStatus::Running);

    let web = names.container("app-test", "web");
    async fn run(engine: &Engine, web: &str, argv: &[&str]) -> ExecOutput {
        let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        engine
            .exec_output(web, &argv, None, Duration::from_secs(10))
            .await
            .unwrap()
    }
    // 容器里读得到，属主和权限照声明。exec 的输出会去首尾空白，换行用文件大小核对。
    let motd = run(&engine, &web, &["cat", "/etc/motd"]).await;
    assert_eq!(motd.exit_code, 0, "{}", motd.stderr);
    assert_eq!(motd.stdout, "v1");
    let stat = run(&engine, &web, &["stat", "-c", "%s %u %g %a", "/etc/motd"]).await;
    assert_eq!(stat.stdout, "3 65532 65532 440");
    // 根文件系统只读，包括 /tmp。
    let write = run(&engine, &web, &["sh", "-c", "echo x > /tmp/w"]).await;
    assert_ne!(write.exit_code, 0, "{}", write.stdout);
    // 宿主上的 store 文件是 daemon 写的，属主和权限照声明——核心以非 root 运行时也成立。
    // 有实例名时，configs 存在 <数据目录>/core/configs/<实例>/<插件>/<服务>/<编号>。
    let staged = dir
        .path()
        .join("core/configs")
        .join(names.name())
        .join("app-test/web/0");
    let meta = std::fs::metadata(&staged).unwrap();
    assert_eq!(
        (meta.uid(), meta.gid(), meta.mode() & 0o777),
        (65532, 65532, 0o440)
    );

    // 内容更新（升级）重建容器后读到新内容。
    let id = PluginId::parse("app.test").unwrap();
    m.upgrade_upload(
        &id,
        PluginPackage::parse(&compose("v2\\n"), &super::langs()).unwrap(),
        vec![],
        Vec::new(),
        false,
        &[],
        &Default::default(),
        &super::langs(),
    )
    .await
    .unwrap();
    let motd = run(&engine, &web, &["cat", "/etc/motd"]).await;
    assert_eq!(motd.stdout, "v2");

    // 卸载后核心写的文件没了。
    m.uninstall(&id, false, &super::langs()).await.unwrap();
    assert!(!staged.exists());
    assert!(m.list().is_empty());
}

/// 没写 `mode` 的配置是 0644：一次性的复制服务复制出去以后，应用和主人都还能改。
#[tokio::test]
#[ignore = "需要 Docker，真实容器验证默认权限"]
async fn configs_are_writable_by_their_owner_so_copies_can_be_edited() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    crate::test_docker::ensure_image(&engine, &image()).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name());
    manager.cfg.install_timeout = Duration::from_secs(60);
    let m = Arc::new(manager);
    let compose = format!(
        "configs:\n  seed:\n    content: \"v1\"\n{}",
        package(
            &format!(
                "  web:\n    image: {}\n    command: ['sleep', 'infinity']\n    \
                 configs: [{{source: seed, target: /seed/app.conf}}]",
                image()
            ),
            "  permissions: []",
        )
    );
    m.install(&compose, &[], super::langs()).await.unwrap();
    let web = names.container("app-test", "web");
    let stat = engine
        .exec_output(
            &web,
            &["stat", "-c", "%a", "/seed/app.conf"]
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>(),
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert_eq!(stat.stdout, "644", "{}", stat.stderr);
    m.uninstall(
        &PluginId::parse("app.test").unwrap(),
        false,
        &super::langs(),
    )
    .await
    .unwrap();
}

/// 目标父目录在镜像里不存在时核心建出来：busybox 里没有 `/seed`，配置照样写得进去。
#[tokio::test]
#[ignore = "需要 Docker，真实容器验证自动建父目录"]
async fn a_missing_parent_directory_is_created_before_the_file_is_written() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    crate::test_docker::ensure_image(&engine, &image()).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name());
    manager.cfg.install_timeout = Duration::from_secs(60);
    let m = Arc::new(manager);
    let compose = format!(
        "configs:\n  seed:\n    content: \"v1\"\n{}",
        package(
            &format!(
                "  web:\n    image: {}\n    command: ['sleep', 'infinity']\n    \
                 configs: [{{source: seed, target: /seed/deep/app.conf}}]",
                image()
            ),
            "  permissions: []",
        )
    );
    m.install(&compose, &[], super::langs()).await.unwrap();
    let web = names.container("app-test", "web");
    let read = engine
        .exec_output(
            &web,
            &["stat", "-c", "%a %U", "/seed/deep/app.conf"]
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>(),
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert_eq!(read.exit_code, 0, "{}", read.stderr);
    assert_eq!(read.stdout, "644 root");
    m.uninstall(
        &PluginId::parse("app.test").unwrap(),
        false,
        &super::langs(),
    )
    .await
    .unwrap();
}
