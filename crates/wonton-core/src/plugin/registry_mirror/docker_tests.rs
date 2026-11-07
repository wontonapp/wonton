use wonton_protocol::RegistryMirrorUpdate;

use super::docker_fixture::Fixture;
use crate::plugin::PluginStatus;
use crate::plugin::languages::Languages;

async fn enable(fixture: &Fixture, mirror: &str) {
    fixture
        .manager
        .set_registry_mirror(
            RegistryMirrorUpdate {
                mirror: Some(mirror.to_owned()),
            },
            &Default::default(),
        )
        .await
        .unwrap();
}

async fn correct_digest(fixture: &mut Fixture) {
    fixture.start().await;
    let digest = fixture.push("good").await;
    fixture.evict_fixture_images().await;
    let original = format!("{}:fixture@{digest}", fixture.repo);
    assert!(
        fixture
            .engine
            .docker
            .inspect_image(&original)
            .await
            .is_err()
    );
    enable(fixture, &fixture.mirror).await;
    let installed = fixture
        .manager
        .install(
            &fixture.compose(&digest),
            &[],
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap();
    assert_eq!(installed.status, PluginStatus::Running);
    let before = fixture
        .engine
        .inspect_container(&fixture.container())
        .await
        .unwrap()
        .unwrap();
    let actual = format!("{}/{}@{digest}", fixture.mirror, fixture.repo);
    assert_eq!(
        before.config.as_ref().unwrap().image.as_deref(),
        Some(actual.as_str())
    );
    let image = fixture.engine.docker.inspect_image(&actual).await.unwrap();
    assert!(
        image
            .repo_digests
            .unwrap()
            .iter()
            .any(|reference| reference.ends_with(&format!("@{digest}")))
    );
    let again = fixture
        .engine
        .ensure_image(
            &original,
            crate::plugin::compose::PullPolicy::Missing,
            Some(&fixture.mirror),
        )
        .await
        .unwrap();
    assert_eq!(again, actual);
    fixture.manager.restore().await.unwrap();
    fixture.manager.restore().await.unwrap();
    let after = fixture
        .engine
        .inspect_container(&fixture.container())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.id, after.id);
    println!("registry mirror: 正确摘要安装 Running，容器使用 {actual}，重复核对容器 ID 未变化");
}

async fn wrong_digest_and_unreachable(fixture: &mut Fixture) {
    fixture.start().await;
    let expected = fixture.push("good").await;
    let wrong_digest = fixture.push("wrong").await;
    assert_ne!(wrong_digest, expected);
    fixture.remove_manifest(&expected).await;
    fixture.evict_fixture_images().await;
    enable(fixture, &fixture.mirror).await;
    let error = fixture
        .manager
        .install(
            &fixture.compose(&expected),
            &[],
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "image_pull_failed");
    assert!(error.to_string().contains(&fixture.mirror), "{error}");
    assert!(
        fixture
            .engine
            .inspect_container(&fixture.container())
            .await
            .unwrap()
            .is_none()
    );
    println!("registry mirror: 错摘要拒绝，不建容器：{error}");
    // 绑住端口但不监听，到测试结束都不放：连过来一定被拒，别的进程也拿不走这个端口。
    let offline_socket = tokio::net::TcpSocket::new_v4().unwrap();
    offline_socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let offline = offline_socket.local_addr().unwrap().to_string();
    enable(fixture, &offline).await;
    let error = fixture
        .manager
        .install(
            &fixture.compose(&expected),
            &[],
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "image_pull_failed");
    assert!(error.to_string().contains(&offline), "{error}");
    assert!(
        fixture
            .engine
            .inspect_container(&fixture.container())
            .await
            .unwrap()
            .is_none()
    );
    println!("registry mirror: 地址不可达拒绝，不建容器：{error}");
}

#[tokio::test]
#[ignore = "需要 Docker，registry:2 验证摘要拉取与容器引用"]
async fn registry_mirror_docker_installs_verified_digest_without_recreating() {
    let mut fixture = Fixture::new();
    correct_digest(&mut fixture).await;
}

#[tokio::test]
#[ignore = "需要 Docker，registry:2 验证错摘要和断网不回退直连"]
async fn registry_mirror_docker_refuses_wrong_content_and_unreachable_mirror() {
    let mut fixture = Fixture::new();
    wrong_digest_and_unreachable(&mut fixture).await;
}
