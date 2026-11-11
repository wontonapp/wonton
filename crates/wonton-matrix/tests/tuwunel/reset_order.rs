//! 重置身份与设备列表刷新并行：新公钥上传之前发出的 keys/query 拿回的是旧身份，不能把刚生成的私钥删掉。

use std::time::Duration;

use matrix_sdk::{
    Client, encryption::CrossSigningResetAuthType, encryption::EncryptionSettings,
    ruma::api::client::uiaa::AuthData,
};
use wonton_matrix::testkit::{TestServer, eventually, unique};
use wonton_matrix::{Recovery, Verification};

const JWT: &str = "org.matrix.login.jwt";

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_query_during_reset_cannot_drop_the_new_identity() {
    let server = TestServer::start().await;
    let user = unique("owner");
    let first = server.login(&user, "first").await;
    first.start_sync().await.unwrap();
    eventually("first identity ready", first.encryption_states(), |state| {
        (state.verification == Verification::Verified).then_some(())
    })
    .await;
    first.enable_recovery().await.unwrap();
    assert_eq!(first.encryption_state().recovery, Recovery::Enabled);

    let second = Client::builder()
        .homeserver_url(&server.url)
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: true,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();
    second
        .matrix_auth()
        .login_custom(
            JWT,
            [("token".to_owned(), server.issuer.login_token(&user).into())]
                .into_iter()
                .collect(),
        )
        .unwrap()
        .await
        .unwrap();
    second
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;
    let encryption = second.encryption();
    let own = second.user_id().unwrap().to_owned();

    // Tuwunel 换身份要过 UIA：拿到句柄时新私钥已在本机，服务器上还是旧公钥。
    let handle = encryption
        .recovery()
        .reset_identity()
        .await
        .unwrap()
        .expect("Tuwunel 要求确认本人");
    let checking = encryption.request_user_identity(&own);
    tokio::pin!(checking);
    // 这时服务器只会答旧身份；查询要等新公钥上传完才发得出去。等不到是正确结果，
    // 这段时间只决定修复前能不能当场抓到删私钥。
    let early = tokio::time::timeout(Duration::from_secs(2), &mut checking).await;
    assert!(early.is_err(), "新公钥上传之前就处理了 keys/query");

    let CrossSigningResetAuthType::Uiaa(info) = handle.auth_type() else {
        panic!("Tuwunel 应该走 UIA：{:?}", handle.auth_type());
    };
    let mut data = serde_json::Map::new();
    data.insert("token".to_owned(), server.issuer.login_token(&user).into());
    let auth = AuthData::new(JWT, info.session.clone(), data).unwrap();
    handle.reset(Some(auth)).await.unwrap();

    let identity = checking.await.unwrap().expect("查到自己的身份");
    assert!(identity.is_verified(), "查到的应是新身份，由本机私钥签过");
    assert!(
        encryption
            .cross_signing_status()
            .await
            .unwrap()
            .is_complete(),
        "刚生成的交叉签名私钥被删了"
    );
}
