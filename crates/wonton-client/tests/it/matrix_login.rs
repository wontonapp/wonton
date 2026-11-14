//! 用现成的 Matrix 账号登录（ADR-0051）：只经由导出给壳的接口驱动，断言服务器列表上的 `matrix_login` 和槽位。
//!
//! 密码登录对着一次性的 Tuwunel（开着密码登录），OIDC 对着本测试里假扮的 homeserver 和授权服务器
//! （`oidc` 子模块）：Tuwunel 只有旧式 SSO，没有 OAuth 2.0 API。

mod oidc;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use wonton_client::{
    Action, ChatAccount, ClientCore, CoreError, MatrixLoginFailure, MatrixLoginMethod,
    MatrixLoginState, MatrixLoginViewModel, MatrixPasswordLogin, Network, SecretString,
    ServersViewModel,
};
use wonton_matrix::testkit::{TestServer, unique};
use wonton_protocol::ServerKind;

const WAIT: Duration = Duration::from_secs(60);

pub(super) fn open(dir: &Path) -> Arc<ClientCore> {
    crate::chat::support::init_test_logging();
    ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap()
}

pub(super) async fn wait_servers(
    core: &ClientCore,
    mut done: impl FnMut(&ServersViewModel) -> bool,
) -> ServersViewModel {
    let subscription = core.subscribe_servers();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if done(&view) {
                return view;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", core.servers()))
}

/// 等登录走到不再是“发现中、认证中、保存中”的那一步。
pub(super) async fn settled(core: &ClientCore) -> MatrixLoginViewModel {
    let view = wait_servers(core, |view| {
        view.matrix_login.as_ref().is_some_and(|login| {
            !matches!(
                login.state,
                MatrixLoginState::Discovering
                    | MatrixLoginState::Authenticating
                    | MatrixLoginState::Saving
            )
        })
    })
    .await;
    view.matrix_login.unwrap()
}

pub(super) fn start(core: &ClientCore, homeserver: &str) {
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::StartMatrixLogin {
            homeserver: Some(homeserver.to_owned()),
        },
    })
    .unwrap();
}

pub(super) async fn slot_dir_gone(dir: &Path, slot: &str) {
    let path = dir.join("servers").join(slot);
    tokio::time::timeout(WAIT, async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} 还在", path.display()));
}

fn password(login_id: &str, user_id: &str, password: &str) -> Action {
    Action::Home {
        action: wonton_protocol::HomeAction::SubmitMatrixPassword {
            login: MatrixPasswordLogin {
                login_id: login_id.to_owned(),
                user_id: user_id.to_owned(),
                password: SecretString::new(password),
                device_name: "测试手机".to_owned(),
            },
        },
    }
}

async fn signed_in_as(core: &ClientCore, user_id: &str) {
    let subscription = core.subscribe_chat();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if matches!(&view.account, ChatAccount::SignedIn { user_id: id, .. } if id == user_id) {
                return;
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("聊天没登录上：{:?}", core.chat().account));
}

async fn password_server() -> TestServer {
    TestServer::container(&[("TUWUNEL_LOGIN_WITH_PASSWORD", "true")]).await
}

/// 填不合法的服务器：直接失败在“地址无效”，不去连，不占槽位。
#[tokio::test]
async fn an_invalid_homeserver_fails_without_touching_the_slots() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let before = core.servers().servers;
    start(&core, "https://alice:secret@example.org");
    let login = settled(&core).await;
    assert_eq!(
        login.state,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::InvalidHomeserver
        }
    );
    assert_eq!(login.homeserver, "https://alice:secret@example.org");
    assert_eq!(core.servers().servers, before);
    slot_dir_gone(dir.path(), "2").await;
}

/// 密码登录成功：新槽位是这个 Matrix 账号、换了过去，聊天直连 homeserver 登上；重开 App 还在。
/// 同一个账号再登一次，不多占一个槽位。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_password_login_adds_the_account_and_switches_to_it() {
    let server = password_server().await;
    let (user_id, secret) = server.register_password_user(&unique("alice")).await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());

    start(&core, &server.url);
    let ready = settled(&core).await;
    assert_eq!(ready.state, MatrixLoginState::Ready);
    assert!(
        ready.methods.contains(&MatrixLoginMethod::Password),
        "{ready:?}"
    );
    core.dispatch(password(&ready.login_id, &user_id, &secret))
        .unwrap();
    let done = settled(&core).await;
    let MatrixLoginState::Succeeded { slot_id, account } = done.state else {
        panic!("登录没成功：{done:?}");
    };
    assert_eq!(account.user_id, user_id);
    let rows = core.servers().servers;
    let row = rows
        .iter()
        .find(|row| row.id == slot_id)
        .expect("新槽位在列表里");
    assert!(row.current);
    assert_eq!(row.kind, ServerKind::Matrix { account });
    signed_in_as(&core, &user_id).await;

    drop(core);
    let core = open(dir.path());
    signed_in_as(&core, &user_id).await;

    start(&core, &server.url);
    let ready = settled(&core).await;
    core.dispatch(password(&ready.login_id, &user_id, &secret))
        .unwrap();
    let again = settled(&core).await;
    assert!(
        matches!(again.state, MatrixLoginState::Succeeded { .. }),
        "{again:?}"
    );
    let matrix_rows = core
        .servers()
        .servers
        .into_iter()
        .filter(|row| matches!(row.kind, ServerKind::Matrix { .. }))
        .count();
    assert_eq!(matrix_rows, 1, "同一个账号只占一个槽位");
    slot_dir_gone(dir.path(), &slot_id).await;
    signed_in_as(&core, &user_id).await;
}

/// 密码错：失败在“凭据错”，列表不变，占的目录删掉；同一个流程不能再交密码，重试要从新的发现开始。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_wrong_password_fails_and_leaves_the_slots_alone() {
    let server = password_server().await;
    let (user_id, _) = server.register_password_user(&unique("bob")).await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let before = core.servers().servers;

    start(&core, &server.url);
    let ready = settled(&core).await;
    core.dispatch(password(&ready.login_id, &user_id, "not-the-password"))
        .unwrap();
    let failed = settled(&core).await;
    assert_eq!(
        failed.state,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::InvalidCredentials
        }
    );
    assert_eq!(core.servers().servers, before);
    slot_dir_gone(dir.path(), "2").await;
    assert!(matches!(
        core.dispatch(password(&ready.login_id, &user_id, "again")),
        Err(CoreError::InvalidInput { .. })
    ));
}

/// 取消：停在“已取消”，列表不变；之后再交密码不收，迟到的操作不能把流程救回来。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cancelling_a_login_keeps_the_slots_and_refuses_late_submissions() {
    let server = password_server().await;
    let (user_id, secret) = server.register_password_user(&unique("carol")).await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let before = core.servers().servers;

    start(&core, &server.url);
    let ready = settled(&core).await;
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::CancelMatrixLogin {
            login_id: ready.login_id.clone(),
        },
    })
    .unwrap();
    assert_eq!(
        core.servers().matrix_login.unwrap().state,
        MatrixLoginState::Cancelled
    );
    assert!(matches!(
        core.dispatch(password(&ready.login_id, &user_id, &secret)),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        core.dispatch(Action::Home {
            action: wonton_protocol::HomeAction::CancelMatrixLogin {
                login_id: "login-somebody-else".into(),
            },
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(core.servers().servers, before);
    slot_dir_gone(dir.path(), "2").await;
}
