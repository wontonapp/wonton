//! 热加载走管理房间命令，不经 Docker 的 kill 接口（Docker 重启后 `unless-stopped` 不再拉起被发过信号的容器）。

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{FailureKind, FederationConfig};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{self, Fake};

const RELOAD_COMMAND: &str = "!admin server reload-config";

/// 接到假 Tuwunel 上的一份联邦配置。安装目录要活着：`FederationConfig` 只存路径。
async fn config() -> (FederationConfig, Fake, tempfile::TempDir, tempfile::TempDir) {
    let (fake, matrix_config) = fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&matrix_config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let install = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(install.path().join("secrets/tuwunel")).unwrap();
    let config = FederationConfig {
        matrix: Arc::new(matrix),
        install_dir: install.path().to_owned(),
        notice: None,
        server_name: matrix_config.server_name,
    };
    (config, fake, state, install)
}

/// 核心的管理账号被邀请进 `#admins:<server>`，连上时已经加入，之后在那里发命令。
#[tokio::test]
async fn reload_is_an_admin_room_command() {
    let (config, fake, _state, _install) = config().await;
    config
        .apply_inner(&BTreeSet::new(), &BTreeSet::new())
        .await
        .unwrap();

    let state = fake.state();
    let sent: Vec<_> = state
        .sent
        .iter()
        .filter(|(_, content)| content["body"] == RELOAD_COMMAND)
        .collect();
    assert_eq!(sent.len(), 1, "{:?}", state.sent);
    assert_eq!(sent[0].0, fake::ADMIN_ROOM_ID);
    // 只有核心的管理账号，没有为热加载另注册的账号。
    assert_eq!(state.registrations, 1);
    assert!(
        state.rooms[fake::ADMIN_ROOM_ID]
            .members
            .contains("@wonton-core:test")
    );
}

/// 管理账号被踢出管理房间：文件照写，结果如实报成热加载失败。
#[tokio::test]
async fn a_missing_admin_membership_is_a_reload_failure() {
    let (config, fake, _state, _install) = config().await;
    fake.state()
        .rooms
        .get_mut(fake::ADMIN_ROOM_ID)
        .unwrap()
        .members
        .remove("@wonton-core:test");
    let failure = config
        .apply_inner(&BTreeSet::new(), &BTreeSet::new())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Reload);
}
