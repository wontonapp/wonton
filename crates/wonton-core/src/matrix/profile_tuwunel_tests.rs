//! MSC4133 必须在真实 homeserver 上验证，不能由 fake 证明服务器支持它。

use std::process::Command;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use wonton_protocol::StateAvatars;

use super::profile_tests::sample_avatars;
use super::{Matrix, MatrixError, Session};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::test_docker::Names;

const IMAGE: &str = "ghcr.io/matrix-construct/tuwunel:v1.9.4";
const CONFIG: &str = include_str!("../../../../deploy/dev/matrix/tuwunel.toml");
const FEDERATION: &str = include_str!("../../../../deploy/dev/matrix/federation.toml");

/// 开发栈的第二份联邦配置和正式安装生成的保持一致（ADR-0050）：都按两份 `-c` 文件组成。
/// 测试用基础配置起 Tuwunel（回环地址上联邦开不了），这份文件只有 dev compose 挂，这里钉住它的选项。
#[test]
fn the_dev_stack_federation_options_match_the_install() {
    for expected in [
        "port = [8008, 8448]",
        "allow_federation = true",
        "trusted_servers = []",
        "ip_range_denylist = []",
        "allow_invalid_tls_certificates = true",
        "[global.tls]",
        "dual_protocol = true",
    ] {
        assert!(
            FEDERATION.contains(expected),
            "{expected} 不在开发栈的联邦配置里：\n{FEDERATION}"
        );
    }
}

struct Tuwunel {
    _dir: tempfile::TempDir,
    config: MatrixConfig,
}

impl Tuwunel {
    /// `name` 由调用方的 test_docker::Names 生成并登记，容器由守卫回收。
    fn start(name: String) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuwunel.toml");
        std::fs::write(&path, CONFIG).unwrap();
        let pair = Ed25519KeyPair::generate().unwrap();
        let private = STANDARD.encode(pair.to_pkcs8().unwrap().as_ref());
        let public = STANDARD.encode(pair.public_key().as_der().unwrap().as_ref());
        let public = format!(
            "TUWUNEL_JWT__KEY=-----BEGIN PUBLIC KEY-----\n{public}\n-----END PUBLIC KEY-----\n"
        );
        // 在 run 之前持有 guard：启动成功但后续初始化失败也必须删掉本测试的容器。
        let mut server = Self {
            _dir: dir,
            config: MatrixConfig {
                url: String::new(),
                server_name: "wonton.localhost".into(),
                jwt_key: format!(
                    "-----BEGIN PRIVATE KEY-----\n{private}\n-----END PRIVATE KEY-----\n"
                ),
                registration_secret: crate::secret::random_token(),
            },
        };
        let mount = format!("{}:/etc/tuwunel/tuwunel.toml:ro", path.display());
        let secret = format!(
            "TUWUNEL_REGISTRATION_SHARED_SECRET={}",
            server.config.registration_secret
        );
        let result = Command::new("docker")
            .args([
                "run",
                "--detach",
                "--name",
                &name,
                "--publish",
                "127.0.0.1::8008",
                "--volume",
                &mount,
                "--env",
                "TUWUNEL_CONFIG=/etc/tuwunel/tuwunel.toml",
                "--env",
                &public,
                "--env",
                &secret,
                IMAGE,
            ])
            .output()
            .expect("运行 Docker");
        assert!(
            result.status.success(),
            "启动 Tuwunel：{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = Command::new("docker")
            .args(["port", &name, "8008/tcp"])
            .output()
            .unwrap();
        assert!(result.status.success(), "读取 Tuwunel 端口失败");
        let address = String::from_utf8(result.stdout).unwrap();
        let address = address.trim();
        assert!(address.starts_with("127.0.0.1:"), "测试只监听回环地址");
        server.config.url = format!("http://{address}");
        server
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，独立 bp-core- 容器验证 Tuwunel v1.9.4 的 MSC4133 和认证媒体"]
async fn tuwunel_bot_profile_fields_round_trip_and_temporary_devices_are_removed() {
    let mut names = Names::new();
    let server = Tuwunel::start(names.oneshot_container("tuwunel"));
    let dir = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&server.config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    let login = matrix.login("profile-bot", "测试插件").await.unwrap();
    let before = matrix.devices(&login.user_id).await.unwrap();
    let avatars = sample_avatars();
    let media = matrix
        .as_bot(&login.user_id, async |bot: &Session<'_>| {
            assert_eq!(bot.avatars().await?, None);
            let url = bot
                .upload_media("image/png", "avatar.png", b"smoke-media".to_vec())
                .await?;
            bot.set_avatar_url(Some(&url)).await?;
            bot.set_display_name("自选名字").await?;
            bot.set_avatars(&avatars).await?;
            assert_eq!(bot.avatar_url().await?, Some(url.clone()));
            assert_eq!(bot.display_name().await?, Some("自选名字".into()));
            assert_eq!(bot.avatars().await?, Some(avatars.clone()));
            Ok(url)
        })
        .await
        .unwrap();
    assert_eq!(matrix.download_media(&media).await.unwrap(), b"smoke-media");
    assert_eq!(matrix.devices(&login.user_id).await.unwrap(), before);
    let result: Result<(), _> = matrix
        .as_bot(&login.user_id, async |bot: &Session<'_>| {
            bot.set_avatars(&StateAvatars::default()).await?;
            assert_eq!(bot.avatars().await?, Some(StateAvatars::default()));
            Err(MatrixError::Invalid("测试闭包失败".into()))
        })
        .await;
    assert!(matches!(result, Err(MatrixError::Invalid(_))));
    assert_eq!(matrix.devices(&login.user_id).await.unwrap(), before);
    matrix
        .as_bot(&login.user_id, async |bot: &Session<'_>| {
            assert_eq!(bot.avatars().await?, Some(StateAvatars::default()));
            bot.delete_avatars().await?;
            assert_eq!(bot.avatars().await?, None);
            bot.set_avatar_url(None).await?;
            assert_eq!(bot.avatar_url().await?, None);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(matrix.devices(&login.user_id).await.unwrap(), before);
}
