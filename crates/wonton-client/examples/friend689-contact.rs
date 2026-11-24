//! 走查夹具（#689）：拿家人的配对码登录成那个普通成员，设好资料，看到邀请等 25 秒再同意，然后挂着。
//!
//! `WONTON_CONTACT_CODE_FILE=<码文件> friend689-contact <空数据目录>`

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wonton_client::{
    Action, ChatAccount, ClientCore, FriendsAction, HomeAction, HomeStatus, PairingStatus,
};
use wonton_protocol::OwnerProfileUpdate;

const WAIT: Duration = Duration::from_secs(240);
const ACCEPT_AFTER: Duration = Duration::from_secs(25);
const DISPLAY_NAME: &str = "689 Contact";

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("要数据目录");
    let path = std::env::var("WONTON_CONTACT_CODE_FILE").expect("要 WONTON_CONTACT_CODE_FILE");
    let code = std::fs::read_to_string(&path).expect("读配对码");
    // 走查时要看配对卡在哪一步：默认 info，`RUST_LOG` 可调高。
    wonton_client::init_logging(
        std::env::var("RUST_LOG")
            .unwrap_or_else(|_| "wonton_client=info,wonton_tunnel=info".to_owned()),
        None,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建不了运行时");
    runtime.block_on(run(&dir, code.trim()));
}

async fn run(dir: &str, code: &str) {
    // 客户端建起来时自己会写 servers.json：必须在建之前看这个目录配过没有。
    // 配过再拿新码 PairHome 会被当成 CodeUsed，直接等它用存下的会话重连。
    let paired = std::fs::read_to_string(format!("{dir}/servers.json"))
        .map(|saved| !saved.trim().is_empty())
        .unwrap_or(false);
    let core: Arc<ClientCore> = ClientCore::new(dir.to_owned()).expect("建客户端");
    let pair_home = || {
        core.dispatch(Action::Home {
            action: HomeAction::PairHome {
                code: wonton_protocol::SecretString::new(code.to_owned()),
                device_name: DISPLAY_NAME.to_owned(),
            },
        })
        .expect("派发 PairHome");
    };
    if !paired {
        pair_home();
    }
    // 加入别人家的码要管理员同意：等批是一个状态，不是失败。同意之后拿同一个码再配一次才会配上，
    // 所以在等批期间自己重试。
    let deadline = Instant::now() + WAIT;
    let mut pending = false;
    loop {
        if !paired && let Some(PairingStatus::JoinPending { .. }) = &core.home().pairing {
            if !pending {
                println!("等管理员同意加入");
                let _ = std::io::stdout().flush();
                pending = true;
            }
            pair_home();
        }
        if matches!(core.home().status, HomeStatus::Connected) {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "等不到：隧道连上家里的服务器（home={:?}）",
                core.home().status
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let user_id = wait_for("登录成家人账号", || match &core.chat().account {
        ChatAccount::SignedIn { user_id, .. } => Some(user_id.clone()),
        _ => None,
    })
    .await;
    // 登录先于连接就绪时 save_me 会报没连上：等到成功为止。
    let mut saved = false;
    for _ in 0..60 {
        match core
            .save_me(OwnerProfileUpdate {
                display_name: DISPLAY_NAME.to_owned(),
                email: None,
            })
            .await
        {
            Ok(_) => {
                saved = true;
                break;
            }
            Err(error) => {
                println!("设置资料还没成功：{error:?}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    assert!(saved, "设置资料");
    println!("CONTACT_READY {user_id}");

    // 等邀请；iOS 和 Android 各发一次，同一个联系人两次都要同意。
    let mut seen = std::collections::HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(40 * 60);
    loop {
        assert!(Instant::now() < deadline, "等不到好友邀请");
        core.dispatch(Action::Friends {
            action: FriendsAction::RefreshFriends,
        })
        .expect("刷新好友页");
        tokio::time::sleep(Duration::from_secs(2)).await;
        let Some(request) = core
            .friends()
            .incoming
            .iter()
            .find(|request| !seen.contains(&request.room_id))
            .cloned()
        else {
            continue;
        };
        seen.insert(request.room_id.clone());
        println!("CONTACT_INVITE {}", request.room_id);
        // 留一段“等待对方同意”的时间给走查截图。
        tokio::time::sleep(ACCEPT_AFTER).await;
        core.dispatch(Action::Friends {
            action: FriendsAction::ApproveFriendRequest {
                room_id: request.room_id,
            },
        })
        .expect("同意邀请");
        println!("CONTACT_ACCEPTED");
    }
}

async fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "等不到：{}", what);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
