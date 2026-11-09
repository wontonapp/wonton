//! 真实 Docker：`/proc/net/tcp` 的直连探测认得出宿主上真的监听（ADR-0036）。

use bollard::models::{ContainerCreateBody, HostConfig};

use super::super::docker::Engine;
use crate::test_docker::Names;

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_real_loopback_listener_is_recognized_in_the_proc_table() {
    use crate::plugin::compose::PullPolicy;

    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    // 本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
    let image = wonton_test_images::image("public.ecr.aws/docker/library/busybox:1.37");
    engine
        .ensure_image(&image, PullPolicy::Missing, &[])
        .await
        .unwrap();
    let mut names = Names::new();
    let name = names.oneshot_container("relay-probe");
    engine
        .create_container(
            &name,
            ContainerCreateBody {
                image: Some(image.clone()),
                // 端口交给 nc 自己挑：测试先找空闲端口再交给它，中间隔着一次 `docker run`，宿主上别的进程能抢走。
                cmd: Some(vec![
                    "sh".to_owned(),
                    "-c".to_owned(),
                    "nc -l -p 0 -s 127.0.0.1".to_owned(),
                ]),
                host_config: Some(HostConfig {
                    network_mode: Some("host".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.start_container(&name).await.unwrap();
    let mut probed = None;
    for _ in 0..100 {
        if let Some(port) = listening_port(&engine, &name).await {
            probed = super::super::host_relay::listens_beyond_loopback(
                &engine,
                &name,
                port,
                &super::super::languages::Languages::default(),
            )
            .await;
            if probed.is_some() {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(probed, Some(false));
}

/// 容器里 `nc` 挑上的端口。
async fn listening_port(engine: &Engine, name: &str) -> Option<u16> {
    let argv = ["netstat", "-ltnp"].map(str::to_owned);
    let output = engine
        .exec_output(name, &argv, None, std::time::Duration::from_secs(3))
        .await
        .ok()?;
    output.stdout.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.last()?.ends_with("/nc").then_some(())?;
        let (_, port) = fields.get(3)?.rsplit_once(':')?;
        port.parse().ok()
    })
}
