//! 名单文件缺失和写不进去时的行为：缺文件照名单新建，失败按类别报。

use std::collections::BTreeSet;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use super::{FILE, FailureKind, FederationConfig, config_toml};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{self, Fake};

const SERVER: &str = "home.example.org";

async fn config(install_dir: &Path) -> (FederationConfig, Fake) {
    let (fake, matrix_config) = fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&matrix_config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let config = FederationConfig {
        matrix: Arc::new(matrix),
        install_dir: install_dir.to_owned(),
        notice: None,
        server_name: matrix_config.server_name,
    };
    (config, fake)
}

fn paired() -> BTreeSet<String> {
    BTreeSet::from(["peer.example.net".to_owned()])
}

#[tokio::test]
async fn a_missing_list_file_is_written_from_the_pairings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("secrets/tuwunel")).unwrap();

    // 假服务器拒收下一条消息：热加载这一步失败；文件那一步不算失败。
    let (config, fake) = config(dir.path()).await;
    fake.state().refuse_next_send = true;
    let failure = config
        .apply_inner(&paired(), &BTreeSet::new())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Reload);

    let path = dir.path().join(FILE);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        config_toml(SERVER, &paired(), &BTreeSet::new())
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn an_existing_list_file_is_rewritten_in_place() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("secrets/tuwunel")).unwrap();
    let path = dir.path().join(FILE);
    std::fs::write(
        &path,
        "旧的名单，比新的长得多，截断后不能留下尾巴。".repeat(20),
    )
    .unwrap();
    let inode = std::fs::metadata(&path).unwrap().ino();

    let (config, _fake) = config(dir.path()).await;
    let _ = config.apply_inner(&paired(), &BTreeSet::new()).await;

    assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        config_toml(SERVER, &paired(), &BTreeSet::new())
    );
}

#[tokio::test]
async fn an_incomplete_install_is_a_write_failure() {
    let dir = tempfile::tempdir().unwrap();

    let (config, _fake) = config(dir.path()).await;
    let failure = config
        .apply_inner(&paired(), &BTreeSet::new())
        .await
        .unwrap_err();

    assert_eq!(failure.kind, FailureKind::Write);
    assert!(!dir.path().join("secrets").exists());
}

#[tokio::test]
async fn an_invalid_server_name_is_reported_before_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("secrets/tuwunel")).unwrap();
    let blocked = BTreeSet::from(["Not A Name".to_owned()]);

    let (config, _fake) = config(dir.path()).await;
    let failure = config
        .apply_inner(&BTreeSet::new(), &blocked)
        .await
        .unwrap_err();

    assert_eq!(failure.kind, FailureKind::InvalidName);
    assert!(!dir.path().join(FILE).exists());
}
