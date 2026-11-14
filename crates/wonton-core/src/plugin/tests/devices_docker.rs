//! 真实 Docker：设备授权的只读配置与 by-id 路径重解析。

use super::langs;

#[tokio::test]
#[ignore = "需要 Docker：真起容器检查设备授权和 by-id 重启解析"]
async fn docker_honors_device_mapping_permissions_and_resolves_by_id_again_on_restart() {
    use std::os::unix::fs::MetadataExt;

    use super::super::docker::Engine;

    let engine = Engine::connect().unwrap();
    let names = crate::test_docker::Names::new();
    let image = crate::test_docker::ensure_image(
        &engine,
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("usb-FTDI-if00-port0");
    std::os::unix::fs::symlink("/dev/null", &link).unwrap();
    let container = names.container("devprobe", "zigbee");
    let body = bollard::models::ContainerCreateBody {
        image: Some(image),
        cmd: Some(vec!["sleep".to_owned(), "300".to_owned()]),
        labels: Some(names.instance().core_labels()),
        host_config: Some(bollard::models::HostConfig {
            devices: Some(vec![
                bollard::models::DeviceMapping {
                    path_on_host: Some(link.to_string_lossy().into_owned()),
                    path_in_container: Some("/dev/zigbee".to_owned()),
                    cgroup_permissions: Some("rwm".to_owned()),
                },
                bollard::models::DeviceMapping {
                    path_on_host: Some("/dev/zero".to_owned()),
                    path_in_container: Some("/dev/readonly-probe".to_owned()),
                    cgroup_permissions: Some("r".to_owned()),
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(&container, body).await.unwrap();
    engine.start_container(&container).await.unwrap();
    let inspected = engine.inspect_container(&container).await.unwrap().unwrap();
    let mappings = inspected.host_config.unwrap().devices.unwrap();
    assert!(
        mappings
            .iter()
            .any(
                |device| device.path_in_container.as_deref() == Some("/dev/readonly-probe")
                    && device.cgroup_permissions.as_deref() == Some("r")
            )
    );
    async fn number(engine: &Engine, container: &str) -> String {
        let out = engine
            .exec_output(
                container,
                &[
                    "stat".to_owned(),
                    "-c".to_owned(),
                    "%t:%T".to_owned(),
                    "/dev/zigbee".to_owned(),
                ],
                None,
                std::time::Duration::from_secs(20),
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        out.stdout.trim().to_owned()
    }
    assert_eq!(
        number(&engine, &container).await,
        "1:3",
        "/dev/null 的设备号"
    );
    // 改指 /dev/full：重启后容器里拿到的必须是新节点。
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/dev/full", &link).unwrap();
    engine
        .restart_container(&container, &langs())
        .await
        .unwrap();
    let full = std::fs::metadata("/dev/full").unwrap();
    assert_eq!(
        number(&engine, &container).await,
        format!("{:x}:{:x}", full.rdev() >> 8, full.rdev() & 0xff)
    );
    engine.remove_container(&container).await.unwrap();
}
