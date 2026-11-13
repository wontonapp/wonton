//! 真实 Tuwunel v2.0.0 把默认邀请过滤当回事：默认值写进账号数据、已有的不覆盖，空名单放行一个人，
//! 名单里加上这个人以后他的邀请被 403 拒掉。Tuwunel 只在 `org.matrix.msc4155.invite_permission_config`
//! 上读名单（`src/service/users/invite_filter.rs` 的 `LISTS_TYPE`）。

use std::process::Command;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;

use super::invite_filter::INVITE_FILTER_TYPE;
use super::{Matrix, MatrixError};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::test_docker::Names;

const TUWUNEL: &str = "ghcr.io/matrix-construct/tuwunel:v2.0.0@sha256:3dde1e9b3503c68218f95415ec3ebff522bd945065ba76019d408543540d1730";
const CONFIG: &str = include_str!("../../../../deploy/dev/matrix/tuwunel.toml");
const SERVER: &str = "filter.test";

fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("运行 Docker");
    assert!(
        output.status.success(),
        "docker {args:?}：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// 起一台 Tuwunel，返回连它的配置（地址是发布到宿主回环上的端口）。
fn tuwunel(names: &mut Names, dir: &std::path::Path) -> MatrixConfig {
    let path = dir.join("tuwunel.toml");
    std::fs::write(&path, CONFIG).unwrap();
    let pair = Ed25519KeyPair::generate().unwrap();
    let private = STANDARD.encode(pair.to_pkcs8().unwrap().as_ref());
    let public = STANDARD.encode(pair.public_key().as_der().unwrap().as_ref());
    let secret = crate::secret::random_token();
    let name = names.oneshot_container("tuwunel");
    docker(&[
        "run",
        "--detach",
        "--name",
        &name,
        "--publish",
        "127.0.0.1::8008",
        "--volume",
        &format!("{}:/etc/tuwunel/tuwunel.toml:ro", path.display()),
        "--env",
        "TUWUNEL_CONFIG=/etc/tuwunel/tuwunel.toml",
        "--env",
        &format!("TUWUNEL_SERVER_NAME={SERVER}"),
        "--env",
        &format!(
            "TUWUNEL_JWT__KEY=-----BEGIN PUBLIC KEY-----\n{public}\n-----END PUBLIC KEY-----\n"
        ),
        "--env",
        &format!("TUWUNEL_REGISTRATION_SHARED_SECRET={secret}"),
        &wonton_test_images::image(TUWUNEL),
    ]);
    let address = docker(&["port", &name, "8008/tcp"]);
    MatrixConfig {
        url: format!("http://{address}"),
        server_name: SERVER.into(),
        jwt_key: format!("-----BEGIN PRIVATE KEY-----\n{private}\n-----END PRIVATE KEY-----\n"),
        registration_secret: secret,
    }
}

#[tokio::test]
#[ignore = "需要 Docker，起真实 Tuwunel v2.0.0 验证默认邀请过滤写进去、不被覆盖、名单生效"]
async fn the_default_filter_is_written_and_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let mut names = Names::new();
    let config = tuwunel(&mut names, dir.path());
    let matrix = Matrix::connect(&config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    let owner = matrix.user_id("owner");

    matrix.ensure_default_invite_filter("owner").await.unwrap();
    assert_eq!(
        matrix.global_account_data(&owner).await.unwrap()[INVITE_FILTER_TYPE],
        json!({"enabled": true})
    );
    assert!(
        matrix.devices(&owner).await.unwrap().is_empty(),
        "临时设备没吊销"
    );

    // 写过的不覆盖。
    let kept = json!({"enabled": true, "blocked_users": ["@spam:elsewhere.test"]});
    let owner_login = matrix.login("owner", "owner").await.unwrap();
    matrix
        .session(&owner_login)
        .put_account_data(INVITE_FILTER_TYPE, &kept)
        .await
        .unwrap();
    matrix.ensure_default_invite_filter("owner").await.unwrap();
    assert_eq!(
        matrix.global_account_data(&owner).await.unwrap()[INVITE_FILTER_TYPE],
        kept
    );

    // 空名单放行陌生人；名单里加上 bob 以后，他的邀请被 Tuwunel 403 拒掉。
    let bob = matrix.login("bob", "bob").await.unwrap();
    let bob_session = matrix.session(&bob);
    let room = bob_session.create_direct_room(&owner).await.unwrap();
    bob_session.invite(&room, &owner).await.unwrap();

    let bob_id = matrix.user_id("bob");
    matrix
        .session(&owner_login)
        .put_account_data(
            INVITE_FILTER_TYPE,
            &json!({"enabled": true, "blocked_users": [bob_id]}),
        )
        .await
        .unwrap();
    let room = bob_session.create_direct_room(&owner).await.unwrap();
    let refused = bob_session.invite(&room, &owner).await.err();
    let Some(MatrixError::Rejected {
        status, errcode, ..
    }) = refused
    else {
        panic!("被屏蔽的邀请没被拒：{refused:?}");
    };
    assert_eq!(status, 403);
    assert!(errcode.ends_with("M_INVITE_BLOCKED"), "{errcode}");
}
