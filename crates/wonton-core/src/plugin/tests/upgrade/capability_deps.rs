use super::*;

#[tokio::test]
async fn upgrade_to_a_version_requiring_capabilities_is_refused() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let before = docker.writes();
    release(
        &github,
        &hello(
            "1.1.0",
            "",
            "  permissions: []\n  requires: [torrent_client]\n",
        ),
    );
    let error = upgrade(&m).await.unwrap_err();
    assert_eq!(error.kind(), "unsupported", "实际错误：{error}");
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(docker.writes(), before);
    assert_eq!(image(&docker, &web()), "web:1.0.0");
}
