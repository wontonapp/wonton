//! 临时端到端探针：两台真云吞之间走加好友、同意、拒绝、删好友、屏蔽和撤销（ADR-0051）。
//!
//! 用法：`friend_e2e --a-dir <目录> --b-dir <目录> --a-code-file <配对码文件A> --b-code-file <配对码文件B>`，也可直接传 `--a-code`、`--b-code`。
//! 两端先各自配对服务器，再把两台服务器互相配对，最后按步骤断言 ViewModel 和房间。失败退出码非零并打印现场。

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wonton_client::{
    Action, ChatAccount, ChatSync, ClientCore, FriendsAction, FriendsState, FriendsViewModel,
    HomeAction, HomeStatus, PeerAction, PeersViewModel,
};

const WAIT: Duration = Duration::from_secs(120);

struct End {
    core: Arc<ClientCore>,
    name: &'static str,
}

impl End {
    fn dispatch(&self, action: Action) {
        self.core
            .dispatch(action)
            .unwrap_or_else(|error| panic!("{}: {error}", self.name));
    }

    async fn signed_in(&self) -> String {
        let subscription = self.core.subscribe_chat();
        wait(self.name, "登录并开始同步", || async {
            let view = subscription.next().await?;
            match (&view.account, &view.sync) {
                (ChatAccount::SignedIn { user_id, .. }, ChatSync::Running) => Some(user_id.clone()),
                _ => None,
            }
        })
        .await
    }

    fn friends(&self) -> FriendsViewModel {
        self.core.friends()
    }

    fn peers(&self) -> PeersViewModel {
        self.core.peers()
    }

    async fn friends_until(
        &self,
        what: &str,
        mut check: impl FnMut(&FriendsViewModel) -> Option<()>,
    ) {
        let subscription = self.core.subscribe_friends();
        let deadline = Instant::now() + WAIT;
        let mut inflight = false;
        loop {
            if self.friends().state != FriendsState::Loading {
                inflight = false;
            }
            if !inflight {
                self.dispatch(Action::Friends {
                    action: FriendsAction::RefreshFriends,
                });
                inflight = true;
            }
            if check(&self.friends()).is_some() {
                println!("PASS: {} {}", self.name, what);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} {} 超时：{:#?}\n聊天：{:#?}",
                self.name,
                what,
                self.friends(),
                self.core.chat().rooms
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = subscription.next() => {}
            }
        }
    }

    async fn peers_until(&self, what: &str, mut check: impl FnMut(&PeersViewModel) -> Option<()>) {
        let subscription = self.core.subscribe_peers();
        let deadline = Instant::now() + WAIT;
        loop {
            self.dispatch(Action::Peers {
                action: PeerAction::RefreshPeers,
            });
            if check(&self.peers()).is_some() {
                println!("PASS: {} {}", self.name, what);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} {} 超时：{:#?}",
                self.name,
                what,
                self.peers()
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = subscription.next() => {}
            }
        }
    }

    fn rooms(&self) -> Vec<String> {
        self.core
            .chat()
            .rooms
            .iter()
            .map(|room| room.room_id.clone())
            .collect()
    }
}

/// 通用等待：在 `WAIT` 内反复调用一个返回 `Option` 的异步函数。
async fn wait<T, F, Fut>(who: &str, what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(Instant::now() < deadline, "{who} {what} 超时");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn read_pair_code(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取配对码文件 {path}: {error}"))
        .trim()
        .to_owned()
}

fn pair_core(dir: &str) -> Arc<ClientCore> {
    ClientCore::new(dir.to_owned()).expect("建客户端")
}

async fn pair(end: &End, code: &str) {
    end.dispatch(Action::Home {
        action: HomeAction::PairHome {
            code: wonton_protocol::SecretString::new(code),
            device_name: format!("{} 的探针", end.name),
        },
    });
    let home = end.core.subscribe_home();
    wait(end.name, "连上家里的服务器", || async {
        let view = home.next().await?;
        (view.status == HomeStatus::Connected).then_some(())
    })
    .await;
    println!("PASS: {} 已配对", end.name);
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    if let Ok(filter) = std::env::var("FRIEND_E2E_LOG") {
        wonton_client::init_logging(filter, None);
    }
    let mut a_dir = None;
    let mut b_dir = None;
    let mut a_code = None;
    let mut b_code = None;
    let mut a_code_file = None;
    let mut b_code_file = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--a-dir" => a_dir = args.next(),
            "--b-dir" => b_dir = args.next(),
            "--a-code" => a_code = args.next(),
            "--b-code" => b_code = args.next(),
            "--a-code-file" => a_code_file = args.next(),
            "--b-code-file" => b_code_file = args.next(),
            other => panic!("不认识的参数 {other}"),
        }
    }
    let a = End {
        core: pair_core(&a_dir.expect("--a-dir")),
        name: "A",
    };
    let b = End {
        core: pair_core(&b_dir.expect("--b-dir")),
        name: "B",
    };
    let a_code = a_code
        .or_else(|| a_code_file.map(|path| read_pair_code(&path)))
        .expect("--a-code 或 --a-code-file");
    let b_code = b_code
        .or_else(|| b_code_file.map(|path| read_pair_code(&path)))
        .expect("--b-code 或 --b-code-file");
    pair(&a, &a_code).await;
    pair(&b, &b_code).await;
    let a_user = a.signed_in().await;
    let b_user = b.signed_in().await;
    println!("A={a_user} B={b_user}");

    // 两台服务器互相配对：联邦只在配对过之间开。核心之间拨号的是扫码方，所以由能连到的那台出码：
    // B 在远端、码里是局域网地址，A 在本机能连；A 的码是 127.0.0.1，B 的核心连不到。
    // 已经配对过（同一对栈重复跑）就跳过。
    let a_server = a_user.rsplit_once(':').expect("用户 ID 有服务器名").1;
    let b_server = b_user.rsplit_once(':').unwrap().1.to_owned();
    a.dispatch(Action::Peers {
        action: PeerAction::RefreshPeers,
    });
    b.dispatch(Action::Peers {
        action: PeerAction::RefreshPeers,
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    // 只有真连上或正在探活才算配过：`RemovedByPeer`、`AwaitingApproval`、`Declined` 都重配。
    println!("A peers: {:#?}\nB peers: {:#?}", a.peers(), b.peers());
    let known = |view: &PeersViewModel, server: &str| {
        view.peers.iter().any(|peer| {
            peer.server_name == server
                && matches!(
                    peer.status,
                    wonton_client::PeerStatus::Connected | wonton_client::PeerStatus::Checking
                )
        })
    };
    if !(known(&a.peers(), &b_server) && known(&b.peers(), a_server)) {
        b.dispatch(Action::Peers {
            action: PeerAction::ShowPeerCode,
        });
        b.peers_until("出码", |view| view.code.clone().map(|_| ()))
            .await;
        let code = b.peers().code.clone().unwrap();
        a.dispatch(Action::Peers {
            action: PeerAction::ScanPeerCode {
                code: code.code.clone(),
            },
        });
        b.peers_until("收到配对请求", |view| {
            (!view.requests.is_empty()).then_some(())
        })
        .await;
        let id = b.peers().requests[0].id.clone();
        b.dispatch(Action::Peers {
            action: PeerAction::ApprovePeerRequest { id },
        });
    }
    let connected = |view: &PeersViewModel, server: &str| {
        view.peers.iter().any(|peer| {
            peer.server_name == server && peer.status == wonton_client::PeerStatus::Connected
        })
    };
    a.peers_until("配对成立", |view| {
        connected(view, &b_server).then_some(())
    })
    .await;
    b.peers_until("配对成立", |view| {
        connected(view, a_server).then_some(())
    })
    .await;
    let b_server = b_server.clone();
    let _ = b_server;
    println!("PASS: 两台服务器已配对");

    // 加好友 → 请求 → 同意。
    b.dispatch(Action::Friends {
        action: FriendsAction::AddFriend {
            user_id: a_user.clone(),
        },
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("B 发邀请后：{:#?}", b.friends());
    a.friends_until("收到好友请求", |view| {
        view.incoming
            .iter()
            .any(|request| request.user_id == b_user)
            .then_some(())
    })
    .await;
    let room = a
        .friends()
        .incoming
        .iter()
        .find(|request| request.user_id == b_user)
        .unwrap()
        .room_id
        .clone();
    a.dispatch(Action::Friends {
        action: FriendsAction::ApproveFriendRequest {
            room_id: room.clone(),
        },
    });
    for (end, other) in [(&a, &b_user), (&b, &a_user)] {
        end.friends_until("成为好友", |view| {
            (view.state == FriendsState::Ready
                && view.friends.iter().any(|friend| &friend.user_id == other))
            .then_some(())
        })
        .await;
    }

    // 删好友：名单没了，私聊留着。
    a.dispatch(Action::Friends {
        action: FriendsAction::RemoveFriend {
            user_id: b_user.clone(),
        },
    });
    a.friends_until("好友删掉", |view| {
        (!view.friends.iter().any(|friend| friend.user_id == b_user)).then_some(())
    })
    .await;
    assert!(a.rooms().iter().any(|id| id == &room), "删好友不应退出私聊");

    // 再邀请一次（新房间）→ 拒绝。
    a.dispatch(Action::Friends {
        action: FriendsAction::AddFriend {
            user_id: b_user.clone(),
        },
    });
    b.friends_until("又收到请求", |view| {
        view.incoming
            .iter()
            .any(|request| request.user_id == a_user)
            .then_some(())
    })
    .await;
    let rejected = b
        .friends()
        .incoming
        .iter()
        .find(|request| request.user_id == a_user)
        .unwrap()
        .room_id
        .clone();
    b.dispatch(Action::Friends {
        action: FriendsAction::RejectFriendRequest { room_id: rejected },
    });
    b.friends_until("请求拒掉", |view| {
        (!view
            .incoming
            .iter()
            .any(|request| request.user_id == a_user))
        .then_some(())
    })
    .await;

    // 屏蔽用户：再邀请就不进请求；撤销后照常。
    b.dispatch(Action::Friends {
        action: FriendsAction::BlockFriend {
            target: wonton_protocol::FriendBlockTarget::User {
                user_id: a_user.clone(),
            },
            scope: wonton_protocol::FriendBlockScope::Invitations,
        },
    });
    b.friends_until("用户已屏蔽", |view| {
        view.blocked
            .iter()
            .any(|block| {
                block.scope == wonton_protocol::FriendBlockScope::Invitations
                    && block.target
                        == wonton_protocol::FriendBlockTarget::User {
                            user_id: a_user.clone(),
                        }
            })
            .then_some(())
    })
    .await;
    a.dispatch(Action::Friends {
        action: FriendsAction::AddFriend {
            user_id: b_user.clone(),
        },
    });
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !b.friends()
            .incoming
            .iter()
            .any(|request| request.user_id == a_user),
        "屏蔽后邀请不该进请求"
    );
    println!("PASS: 屏蔽后邀请进不来");
    b.dispatch(Action::Friends {
        action: FriendsAction::UnblockFriend {
            target: wonton_protocol::FriendBlockTarget::User {
                user_id: a_user.clone(),
            },
            scope: wonton_protocol::FriendBlockScope::Invitations,
        },
    });
    b.friends_until("用户已撤销", |view| {
        (!view.blocked.iter().any(|block| {
            block.scope == wonton_protocol::FriendBlockScope::Invitations
                && block.target
                    == wonton_protocol::FriendBlockTarget::User {
                        user_id: a_user.clone(),
                    }
        }))
        .then_some(())
    })
    .await;

    // 拉黑整台服务器：同样进不来；撤销。
    let server_a = a_user.rsplit_once(':').expect("用户 ID 有服务器名").1;
    b.dispatch(Action::Friends {
        action: FriendsAction::BlockFriend {
            target: wonton_protocol::FriendBlockTarget::Server {
                server_name: server_a.to_owned(),
            },
            scope: wonton_protocol::FriendBlockScope::Federation,
        },
    });
    b.friends_until("服务器已拉黑", |view| {
        view.blocked
            .iter()
            .any(|block| block.scope == wonton_protocol::FriendBlockScope::Federation)
            .then_some(())
    })
    .await;
    a.dispatch(Action::Friends {
        action: FriendsAction::AddFriend {
            user_id: b_user.clone(),
        },
    });
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !b.friends()
            .incoming
            .iter()
            .any(|request| request.user_id == a_user),
        "拉黑服务器后邀请不该进请求"
    );
    println!("PASS: 拉黑服务器后邀请进不来");
    b.dispatch(Action::Friends {
        action: FriendsAction::UnblockFriend {
            target: wonton_protocol::FriendBlockTarget::Server {
                server_name: server_a.to_owned(),
            },
            scope: wonton_protocol::FriendBlockScope::Federation,
        },
    });
    b.friends_until("服务器已撤销", |view| {
        (!view
            .blocked
            .iter()
            .any(|block| block.scope == wonton_protocol::FriendBlockScope::Federation))
        .then_some(())
    })
    .await;

    println!("PASS: 全部步骤完成");
}
