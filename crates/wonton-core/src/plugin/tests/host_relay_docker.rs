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
    const IMAGE: &str = "public.ecr.aws/docker/library/busybox:1.37";
    engine
        .ensure_image(IMAGE, PullPolicy::Missing, None)
        .await
        .unwrap();
    let mut names = Names::new();
    let port = super::host_relay::free_port();
    let name = names.oneshot_container("relay-probe");
    engine
        .create_container(
            &name,
            ContainerCreateBody {
                image: Some(IMAGE.to_owned()),
                cmd: Some(vec![
                    "sh".to_owned(),
                    "-c".to_owned(),
                    format!("nc -l -p {port} -s 127.0.0.1"),
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
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(probed, Some(false));
}
