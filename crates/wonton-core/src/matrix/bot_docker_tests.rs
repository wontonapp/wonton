//! 真实 Tuwunel 上的 Bot 主房间：谁建、谁邀请、重装后邀请人是谁。Hermes 的 Matrix 适配器只认
//! `MATRIX_ALLOWED_USERS`（归属人）发的邀请，邀请人是 Bot 自己时会把邀请丢掉。

use std::collections::BTreeSet;
use std::process::Command;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::{BotAccount, BotSpec, Matrix, ProvisionedBot, Session};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::plugin::languages::Languages;
use crate::test_docker::Names;

fn image() -> String {
    wonton_test_images::image("ghcr.io/matrix-construct/tuwunel:v2.0.0")
}
const CONFIG: &str = include_str!("../../../../deploy/dev/matrix/tuwunel.toml");

struct Tuwunel {
    _dir: tempfile::TempDir,
    config: MatrixConfig,
}

impl Tuwunel {
    fn start(name: String) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuwunel.toml");
        std::fs::write(&path, CONFIG).unwrap();
        let pair = Ed25519KeyPair::generate().unwrap();
        let private = STANDARD.encode(pair.to_pkcs8().unwrap().as_ref());
        let public = STANDARD.encode(pair.public_key().as_der().unwrap().as_ref());
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
        let result = Command::new("docker")
            .args([
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
                &format!(
                    "TUWUNEL_JWT__KEY=-----BEGIN PUBLIC KEY-----\n{public}\n-----END PUBLIC KEY-----\n"
                ),
                "--env",
                &format!(
                    "TUWUNEL_REGISTRATION_SHARED_SECRET={}",
                    server.config.registration_secret
                ),
                image().as_str(),
            ])
            .output()
            .expect("运行 Docker");
        assert!(
            result.status.success(),
            "启动 Tuwunel：{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let out = Command::new("docker")
            .args(["port", &name, "8008/tcp"])
            .output()
            .unwrap();
        assert!(out.status.success(), "读取 Tuwunel 端口失败");
        let address = String::from_utf8(out.stdout).unwrap();
        server.config.url = format!("http://{}", address.trim());
        server
    }
}

fn spec<'a>(localpart: &'a str, taken: &'a BTreeSet<String>) -> BotSpec<'a> {
    BotSpec {
        localpart,
        taken,
        previous: None,
        device_name: "Wonton 插件 com.nousresearch.hermes",
        display_name: "Hermes",
        avatar: None,
        owner: "owner",
    }
}

/// 房间的全部状态事件。
async fn state_events(bot: &Session<'_>, room: &str) -> Vec<serde_json::Value> {
    let path = format!(
        "/_matrix/client/v3/rooms/{}/state",
        super::api::encode(room)
    );
    let value = bot
        .json(hyper::Method::GET, &path, None, "读房间状态")
        .await
        .unwrap();
    value.as_array().cloned().unwrap_or_default()
}

/// 第二次安装：调用生产方法建房，和 `prepare_bot` 走同一条路。
async fn second_install(matrix: &Matrix) -> ProvisionedBot {
    let login = matrix.login("hermes-2", "重装").await.unwrap();
    let bot = Session::new(&matrix.api, &login.user_id, &login.access_token);
    let room = matrix
        .create_bot_home(&bot, "owner", &matrix.user_id("owner"))
        .await
        .unwrap();
    // 方法从建房一路做到主人退出，邀请已经被 Bot 接受，只能从房间历史里翻那条邀请。
    let senders = invite_senders(&bot, &room).await;
    assert_eq!(senders, [matrix.user_id("owner")], "邀请得是主人发的");
    assert_ne!(senders[0], bot.user_id(), "不是自己邀请自己");
    ProvisionedBot {
        account: BotAccount {
            user_id: login.user_id,
            home_room: room,
        },
        device_id: login.device_id,
        access_token: login.access_token,
        owner: matrix.user_id("owner"),
    }
}

/// 房间里出现过的给 Bot 的邀请的发送人（邀请被接受后成员状态变成 join，只能从历史里翻）。
async fn invite_senders(bot: &Session<'_>, room: &str) -> Vec<String> {
    let path = format!(
        "/_matrix/client/v3/rooms/{}/messages?dir=b&limit=50",
        super::api::encode(room)
    );
    let value = bot
        .json(hyper::Method::GET, &path, None, "读房间历史")
        .await
        .unwrap();
    value["chunk"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["type"] == "m.room.member" && e["content"]["membership"] == "invite")
        .filter_map(|e| e["sender"].as_str().map(str::to_owned))
        .collect()
}

/// 房间状态里每个成员状态的发送人。
async fn member_senders(bot: &Session<'_>, room: &str) -> Vec<(String, String, String)> {
    state_events(bot, room)
        .await
        .iter()
        .filter(|event| event["type"] == "m.room.member")
        .map(|event| {
            (
                event["sender"].as_str().unwrap_or_default().to_owned(),
                event["state_key"].as_str().unwrap_or_default().to_owned(),
                event["content"]["membership"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，起真实 Tuwunel v2.0.0 看重装后是谁把 Bot 拉进主房间"]
async fn a_reinstalled_bot_is_invited_by_someone_other_than_itself() {
    let mut names = Names::new();
    let server = Tuwunel::start(names.oneshot_container("tuwunel"));
    let dir = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&server.config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    let taken = BTreeSet::new();
    let first = matrix.provision_bot(&spec("hermes", &taken)).await.unwrap();
    matrix
        .hand_over("owner", &first, &Languages::default())
        .await
        .unwrap();
    // 卸载 --purge：吊销设备、停用账号，房间里的人都被退出。
    matrix
        .delete_device(&first.account.user_id, &first.device_id)
        .await
        .unwrap();
    matrix.deactivate(&first.account.user_id).await.unwrap();
    let second = second_install(&matrix).await;
    assert_eq!(second.account.user_id, "@hermes-2:wonton.localhost");
    assert_ne!(
        second.account.home_room, first.account.home_room,
        "卸载删掉的账号的房间不能再交出去，重装是新房间"
    );
    // 主人在建房之后退出了（不在房间里），交付时得把他请回来。
    assert!(
        matrix
            .hand_over("owner", &second, &Languages::default())
            .await
            .unwrap(),
        "交出没交过的房间"
    );
    let bot = Session::new(&matrix.api, &second.account.user_id, &second.access_token);
    let senders = member_senders(&bot, &second.account.home_room).await;
    let own = senders
        .iter()
        .filter(|(_, who, _)| who == &second.account.user_id)
        .collect::<Vec<_>>();
    assert_eq!(
        own.len(),
        1,
        "Bot 在主房间里只有一条自己的成员状态：{senders:#?}"
    );
    assert_eq!(own[0].2, "join", "重装后 Bot 在主房间里：{senders:#?}");
    // 进房之后加入事件的发送人永远是 Bot 自己；上面那条邀请事件才是邀请人的证据，这里看房间本身的形状。
    let state = state_events(&bot, &second.account.home_room).await;
    let create = state
        .iter()
        .find(|event| event["type"] == "m.room.create")
        .expect("房间有建房事件");
    assert_eq!(
        create["sender"], second.owner,
        "主房间得是主人建的，邀请才不是 Bot 自己发的：{state:#?}"
    );
    assert_eq!(
        create["content"]["additional_creators"][0], second.account.user_id,
        "Bot 也是建房的人，权限和主人一样：{create:#?}"
    );
    // 适配器只处理同步里 rooms.invite 的邀请；停在邀请里的房间会被它按邀请人筛掉。
    let value = bot
        .json(
            hyper::Method::GET,
            "/_matrix/client/v3/sync?timeout=0&set_presence=offline",
            None,
            "同步",
        )
        .await
        .unwrap();
    let invites = value["rooms"]["invite"].as_object();
    assert!(
        invites.is_none_or(|invites| !invites.contains_key(&second.account.home_room)),
        "房间已经进了就不该再是待接受邀请：{invites:#?}"
    );
    assert!(
        value["rooms"]["join"]
            .as_object()
            .is_some_and(|join| join.contains_key(&second.account.home_room)),
        "主房间在 Bot 的已加入房间里"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，起真实 Tuwunel v2.0.0 看撤回主房间能不能把主人弄出去"]
async fn withdrawing_a_handed_over_room_removes_the_owner() {
    let mut names = Names::new();
    let server = Tuwunel::start(names.oneshot_container("tuwunel"));
    let dir = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&server.config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    let taken = BTreeSet::new();
    let bot = matrix.provision_bot(&spec("hermes", &taken)).await.unwrap();
    matrix
        .hand_over("owner", &bot, &Languages::default())
        .await
        .unwrap();
    matrix.withdraw(&bot, &Languages::default()).await.unwrap();
    let session = Session::new(&matrix.api, &bot.account.user_id, &bot.access_token);
    let room = &bot.account.home_room;
    assert_eq!(
        session
            .membership(room, &bot.owner)
            .await
            .unwrap()
            .as_deref(),
        Some("leave"),
        "撤回后主人已经不在房间里"
    );
    assert_eq!(
        session
            .membership(room, &bot.account.user_id)
            .await
            .unwrap()
            .as_deref(),
        Some("leave"),
        "撤回后 Bot 也退出了"
    );
}
