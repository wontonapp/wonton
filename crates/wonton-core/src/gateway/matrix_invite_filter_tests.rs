//! 新账号默认开邀请过滤（MSC4155，ADR-0051 的“挡垃圾”）：第一次换登录令牌前写默认的账号数据，
//! 已经写过的不覆盖，Bot 的账号不写。真实 Tuwunel 收不收这份数据见 `matrix/invite_filter_docker_tests.rs`。

use std::collections::BTreeSet;
use std::sync::Arc;

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use super::tests::CORE;
use super::tunnel_tests::{Home, Phone};
use crate::http::HttpClient;
use crate::matrix::invite_filter::INVITE_FILTER_TYPE;
use crate::matrix::{BotSpec, Matrix, fake};

const OWNER: &str = "@owner:test";

async fn home(dir: &tempfile::TempDir) -> (fake::Fake, Arc<Matrix>, Home) {
    let (fake, config) = fake::start().await;
    let matrix = Arc::new(
        Matrix::connect(&config, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let home = Home::start_with(None, Some(matrix.clone())).await;
    (fake, matrix, home)
}

async fn phone(home: &Home) -> Phone {
    let device = SecretKey::generate();
    home.register(&device).await;
    Phone::start(device, home).await
}

fn stored(fake: &fake::Fake) -> Option<serde_json::Value> {
    fake.state()
        .account_data
        .get(&(OWNER.to_owned(), INVITE_FILTER_TYPE.to_owned()))
        .cloned()
}

/// 第一次换登录令牌时写默认值：过滤开着、名单为空；替账号登录的临时设备写完就吊销。
#[tokio::test]
async fn first_login_writes_the_default_filter() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, _, home) = home(&dir).await;
    let phone = phone(&home).await;

    let (status, _, body) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&fake), Some(json!({"enabled": true})));
    assert!(fake.state().devices(OWNER).is_empty(), "临时设备没吊销");
}

/// 账号自己（或别家客户端）写过的不动：再登录一次，名单里的东西还在。
#[tokio::test]
async fn an_existing_filter_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, _, home) = home(&dir).await;
    let phone = phone(&home).await;
    let written = json!({"enabled": true, "blocked_users": ["@spam:test"]});
    fake.state().account_data.insert(
        (OWNER.to_owned(), INVITE_FILTER_TYPE.to_owned()),
        written.clone(),
    );

    let (status, _, body) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&fake), Some(written));
    assert!(
        fake.state().devices(OWNER).is_empty(),
        "已有数据时不该替账号登录设备"
    );
}

/// Bot 的账号不写：个人屏蔽按人管，Bot 的邀请走 Bot 自己的路径。
#[tokio::test]
async fn bot_accounts_get_no_default() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, matrix, _home) = home(&dir).await;
    let taken = BTreeSet::new();
    let bot = matrix
        .provision_bot(&BotSpec {
            localpart: "hermes",
            taken: &taken,
            previous: None,
            device_name: "test",
            display_name: "艾米",
            avatar: None,
            owner: "owner",
        })
        .await
        .unwrap();

    assert!(
        !fake
            .state()
            .account_data
            .contains_key(&(bot.account.user_id.clone(), INVITE_FILTER_TYPE.to_owned()))
    );
}
