//! 主人自己的资料。

use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use crate::gateway::tunnel_tests::Home;

use super::support::*;

/// 命令行带主人令牌读写主人自己的资料，`dev.sh up` 替主人填资料就这么调。
#[tokio::test]
async fn the_owner_token_reads_and_saves_the_owner_profile() {
    let home = Home::start(None).await;
    let (status, me) = owner_call(&home, Method::GET, ME, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["is_owner"], true);

    let profile = json!({"display_name": "Owner", "email": "changed@example.com"});
    let (status, me) = owner_call(&home, Method::PUT, ME, profile).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let (_, me) = owner_call(&home, Method::GET, ME, Value::Null).await;
    assert_eq!(me["display_name"], "Owner");
    assert_eq!(me["email"], "changed@example.com");
}
