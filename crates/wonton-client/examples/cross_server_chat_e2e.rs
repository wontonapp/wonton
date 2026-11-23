//! 两台真云吞之间的跨服务器加密聊天端到端（#26 C6，ADR-0050、ADR-0051、ADR-0090）。
//!
//! 顺次：两端各自配对设备 → A 在服务器还没配对时直接给 B 上的人发消息（应报“对方服务器没配对”）
//! → 关掉、打开 B 的公网访问总开关 → 两台服务器配对 → 加好友、同意 → A 发起私聊并断言房间开了端到端加密
//! → A 发一句，B 的时间线出现同一句明文 → 暂停 B 的核心，A 再发三句，恢复后 B 在 120 秒内补齐并解出明文。
//!
//! 每步打印 `STEP <名字>: PASS|FAIL <证据>`，末尾给汇总，有一步失败就以 1 退出。用法见
//! deploy/dev/core/smoke-federation.sh；`WONTON_E2E_LOG` 打开客户端日志。
//!
//! main 上 C2/C3/C4 还没合：`public-off`、`unpaired-chat` 和从“好友邀请”往后的步骤会红，这正是本探针的用途。

#![recursion_limit = "256"]

use std::collections::HashMap;
use std::future::Future;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Method;
use reqwest::StatusCode;
use serde_json::Value;

use wonton_client::{
    Action, ChatAccount, ChatAction, ChatSync, ClientCore, FriendsAction, FriendsState,
    FriendsViewModel, HomeAction, HomeStatus, MessageContent, PeerAction, PeerStatus,
    PeersViewModel, SecretString, SendState, TimelineAction, TimelineContent, TimelineDiff,
    TimelineSubscription, TimelineTarget, TimelineUpdate,
};

/// 等一件事的上限。离线补到的窗口按研究定成 120 秒，其余步骤用同一个上限。
const WAIT: Duration = Duration::from_secs(120);

/// 开发栈的域名（`*.wonton.localhost`）解析时先给 `::1`，栈只把端口发布在 `127.0.0.1`：连 IPv4，
/// 原来的 host:port 放进显式 `Host` 头，网关照样按它选路。
fn route(api: &str) -> (String, Option<String>) {
    let api = api.trim_end_matches('/').to_owned();
    let Some((scheme, rest)) = api.split_once("://") else {
        return (api, None);
    };
    let (authority, path) = match rest.find('/') {
        Some(cut) => (&rest[..cut], &rest[cut..]),
        None => (rest, ""),
    };
    let Some((host, port)) = authority.rsplit_once(':') else {
        return (api, None);
    };
    if !host.ends_with(".localhost") {
        return (api, None);
    }
    (
        format!("{scheme}://127.0.0.1:{port}{path}"),
        Some(authority.to_owned()),
    )
}

struct End {
    core: Arc<ClientCore>,
    name: &'static str,
    api: String,
    /// 网关按 Host 选路：连接改用 IPv4 的 127.0.0.1 时，这个头保留栈原来的域名和端口。
    host: Option<String>,
    token: String,
    http: reqwest::Client,
}

impl End {
    fn new(name: &'static str, dir: &str, api: &str, token_file: &str) -> Self {
        let token = std::fs::read_to_string(token_file)
            .unwrap_or_else(|error| panic!("{name} 读令牌 {token_file}: {error}"))
            .trim()
            .to_owned();
        let (api, host) = route(api);
        Self {
            core: ClientCore::new(dir.to_owned())
                .unwrap_or_else(|error| panic!("{name} 建客户端: {error}")),
            name,
            api,
            host,
            token,
            http: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("建 HTTP 客户端"),
        }
    }

    fn dispatch(&self, action: Action) -> Result<(), String> {
        self.core
            .dispatch(action)
            .map_err(|error| format!("{}: {error}", self.name))
    }

    fn chat(&self) -> wonton_client::ChatViewModel {
        self.core.chat()
    }

    fn friends(&self) -> FriendsViewModel {
        self.core.friends()
    }

    fn peers(&self) -> PeersViewModel {
        self.core.peers()
    }

    /// 带主人令牌和第三方客户端头调核心接口（`/_wonton/clients/*`、`/_wonton/api/members`）。
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(StatusCode, Vec<u8>), String> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.api))
            .timeout(WAIT)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Wonton-Clients", "1");
        if let Some(host) = &self.host {
            request = request.header(reqwest::header::HOST, host);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("{} {path}: {error}", self.name))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("{} {path} 读响应: {error}", self.name))?
            .to_vec();
        Ok((status, bytes))
    }

    async fn signed_in(&self) -> Option<String> {
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

    fn server(&self, user_id: &str) -> String {
        user_id
            .rsplit_once(':')
            .unwrap_or_else(|| panic!("{} 的用户 ID {user_id} 没有服务器名", self.name))
            .1
            .to_owned()
    }

    async fn peers_until(
        &self,
        what: &str,
        mut check: impl FnMut(&PeersViewModel) -> Option<()>,
    ) -> Result<(), String> {
        let subscription = self.core.subscribe_peers();
        let deadline = Instant::now() + WAIT;
        loop {
            self.dispatch(Action::Peers {
                action: PeerAction::RefreshPeers,
            })?;
            if check(&self.peers()).is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("{} {what} 超时：{:#?}", self.name, self.peers()));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = subscription.next() => {}
            }
        }
    }

    async fn friends_until(
        &self,
        what: &str,
        mut check: impl FnMut(&FriendsViewModel) -> Option<()>,
    ) -> Result<(), String> {
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
                })?;
                inflight = true;
            }
            if check(&self.friends()).is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{} {what} 超时：好友 {:#?}\n问题：{:#?}",
                    self.name,
                    self.friends(),
                    self.chat().problem
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = subscription.next() => {}
            }
        }
    }

    /// 订阅一个房间的主时间线，收下的明文都记进 [`Timeline::texts`]。
    async fn watch(&self, room: &str) -> Result<Timeline, String> {
        let target = TimelineTarget {
            room_id: room.to_owned(),
            thread_root: None,
        };
        let subscription = self
            .core
            .subscribe_timeline(target.clone())
            .await
            .map_err(|error| format!("{} 打开时间线: {error}", self.name))?;
        Ok(Timeline {
            core: Arc::clone(&self.core),
            name: self.name,
            target,
            subscription,
            texts: HashMap::new(),
        })
    }
}

/// 一个房间的主时间线订阅，按正文记下每一条在服务器上被收下了没有。
struct Timeline {
    core: Arc<ClientCore>,
    name: &'static str,
    target: TimelineTarget,
    subscription: Arc<TimelineSubscription>,
    texts: HashMap<String, bool>,
}

impl Timeline {
    fn send(&self, body: &str) -> Result<(), String> {
        self.core
            .dispatch(Action::Timeline {
                action: TimelineAction::SendText {
                    target: self.target.clone(),
                    body: body.to_owned(),
                    reply_to: None,
                    mentions: Vec::new(),
                },
            })
            .map_err(|error| format!("{} 发送: {error}", self.name))
    }

    fn absorb(&mut self, update: TimelineUpdate) {
        for diff in update.diffs {
            let items = match diff {
                TimelineDiff::Append { items } | TimelineDiff::Reset { items } => items,
                TimelineDiff::PushBack { item } | TimelineDiff::PushFront { item } => vec![item],
                TimelineDiff::Insert { item, .. } | TimelineDiff::Set { item, .. } => vec![item],
                TimelineDiff::Clear
                | TimelineDiff::PopFront
                | TimelineDiff::PopBack
                | TimelineDiff::Remove { .. }
                | TimelineDiff::Truncate { .. } => Vec::new(),
            };
            for item in items {
                let TimelineContent::Message { message } = item.content else {
                    continue;
                };
                let MessageContent::Text { body, .. } = message.body else {
                    continue;
                };
                let sent = matches!(message.send_state, SendState::Sent);
                self.texts
                    .entry(body)
                    .and_modify(|known| *known |= sent)
                    .or_insert(sent);
            }
        }
    }

    /// 等到 `bodies` 都在；`sent` 为真时还要等服务器收下。
    async fn until(&mut self, what: &str, bodies: &[&str], sent: bool) -> Result<(), String> {
        let deadline = Instant::now() + WAIT;
        loop {
            if bodies
                .iter()
                .all(|body| self.texts.get(*body).is_some_and(|known| !sent || *known))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let mut seen: Vec<&String> = self.texts.keys().collect();
                seen.sort();
                return Err(format!(
                    "{} {what} 120 秒超时，没等到：{bodies:?}\n已看到：{seen:#?}",
                    self.name
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                update = self.subscription.next() => {
                    if let Some(update) = update {
                        self.absorb(update);
                    }
                }
            }
        }
    }
}

/// 通用等待：在 `WAIT` 内反复调用一个返回 `Option` 的异步函数。
async fn wait<T, F, Fut>(who: &str, what: &str, mut check: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + WAIT;
    loop {
        // 订阅没有下一批时 next() 会一直挂着，光靠它等不到超时：和 500 毫秒的钟赛跑。
        tokio::select! {
            found = check() => {
                if let Some(found) = found {
                    return Some(found);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        if Instant::now() >= deadline {
            println!("{who} {what} 超时");
            return None;
        }
    }
}

fn read_pair_code(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取配对码文件 {path}: {error}"))
        .trim()
        .to_owned()
}

async fn pair(end: &End, code: &str) -> Result<(), String> {
    end.dispatch(Action::Home {
        action: HomeAction::PairHome {
            code: SecretString::new(code),
            device_name: format!("{} 的探针", end.name),
        },
    })?;
    let home = end.core.subscribe_home();
    let connected = wait(end.name, "连上家里的服务器", || async {
        let view = home.next().await?;
        (view.status == HomeStatus::Connected).then_some(())
    })
    .await;
    connected.ok_or_else(|| format!("{} 没连上家里的服务器", end.name))
}

/// 失败类别的线上名字（`ChatFailure` 的 serde 取值），这样 C1 加了新类别也不用改这里。
fn reason_name(failure: &wonton_client::ChatFailure) -> String {
    serde_json_name(failure).unwrap_or_else(|| format!("{failure:?}"))
}

fn serde_json_name<T: serde::Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn docker(args: &[&str]) -> Result<String, String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|error| format!("跑 docker {args:?}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "docker {args:?} 退出 {:?}：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Default)]
struct Report {
    passed: usize,
    failed: usize,
}

impl Report {
    async fn run<F>(&mut self, step: &str, detail: &str, fut: F) -> bool
    where
        F: Future<Output = Result<String, String>>,
    {
        match fut.await {
            Ok(extra) => {
                self.passed += 1;
                let detail = if extra.is_empty() {
                    detail.to_owned()
                } else {
                    format!("{detail}（{extra}）")
                };
                println!("STEP {step}: PASS {detail}");
                true
            }
            Err(error) => {
                self.failed += 1;
                println!("STEP {step}: FAIL {detail}：{error}");
                false
            }
        }
    }
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    if let Ok(filter) = std::env::var("WONTON_E2E_LOG") {
        wonton_client::init_logging(filter, None);
    }
    let mut a_dir = None;
    let mut b_dir = None;
    let mut a_code = None;
    let mut b_code = None;
    let mut a_api = None;
    let mut b_api = None;
    let mut a_token = None;
    let mut b_token = None;
    let mut b_core = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--a-dir" => a_dir = args.next(),
            "--b-dir" => b_dir = args.next(),
            "--a-code-file" => a_code = args.next(),
            "--b-code-file" => b_code = args.next(),
            "--a-api" => a_api = args.next(),
            "--b-api" => b_api = args.next(),
            "--a-token-file" => a_token = args.next(),
            "--b-token-file" => b_token = args.next(),
            "--b-core" => b_core = args.next(),
            other => panic!("不认识的参数 {other}"),
        }
    }
    let a = End::new(
        "A",
        &a_dir.expect("--a-dir"),
        &a_api.expect("--a-api"),
        &a_token.expect("--a-token-file"),
    );
    let b = End::new(
        "B",
        &b_dir.expect("--b-dir"),
        &b_api.expect("--b-api"),
        &b_token.expect("--b-token-file"),
    );
    let b_core = b_core.expect("--b-core（B 的核心容器名，暂停用）");
    let a_code = read_pair_code(&a_code.expect("--a-code-file"));
    let b_code = read_pair_code(&b_code.expect("--b-code-file"));

    let mut report = Report::default();
    let mut users = None;
    report
        .run("1 pair-homes", "两端各自配对并登录", async {
            pair(&a, &a_code).await?;
            pair(&b, &b_code).await?;
            let a_user = a.signed_in().await.ok_or("A 没登录上")?;
            let b_user = b.signed_in().await.ok_or("B 没登录上")?;
            println!("A={a_user} B={b_user}");
            users = Some((a_user, b_user));
            Ok(String::new())
        })
        .await;
    let Some((a_user, b_user)) = users else {
        finish(&report);
        return;
    };
    let a_server = a.server(&a_user);
    let b_server = b.server(&b_user);

    // B 上另建一个没人聊过的家人：服务器没配对时点“聊天”用它，不会撞上主流程的私聊。
    let probe_user = match b
        .call(
            Method::POST,
            "/_wonton/api/members",
            Some(&serde_json::json!({
                "display_name": "配对探针",
                "username": "pairprobe",
                "private_bot": false,
            })),
        )
        .await
    {
        Ok((status, bytes)) if status == StatusCode::CREATED => {
            let member: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            member["user_id"].as_str().map(str::to_owned)
        }
        other => {
            println!("建家人失败：{other:?}");
            None
        }
    };
    let probe_user = probe_user.unwrap_or_else(|| format!("@pairprobe:{b_server}"));

    report
        .run("2 peers-unpaired", "两台服务器还没配对", async {
            for end in [&a, &b] {
                end.dispatch(Action::Peers {
                    action: PeerAction::RefreshPeers,
                })?;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            for (end, server) in [(&a, b_server.as_str()), (&b, a_server.as_str())] {
                let known = end.peers().peers.iter().any(|peer| {
                    peer.server_name == server
                        && matches!(peer.status, PeerStatus::Connected | PeerStatus::Checking)
                });
                if known {
                    return Err(format!("{} 已经知道 {server}", end.name));
                }
            }
            Ok(String::new())
        })
        .await;

    report
        .run("3 public-off", "B 的公网总开关关掉并读回", async {
            let (status, bytes) = b
                .call(
                    Method::PUT,
                    "/_wonton/clients/public-access",
                    Some(&serde_json::json!({ "enabled": false })),
                )
                .await?;
            if status != StatusCode::OK {
                return Err(format!(
                    "PUT public-access 回 {status}：{}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
            let (status, bytes) = b.call(Method::GET, "/_wonton/clients/status", None).await?;
            if status != StatusCode::OK {
                return Err(format!(
                    "GET clients/status 回 {status}：{}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
            let status: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if status["public_access"]["enabled"] != Value::Bool(false) {
                return Err(format!("状态里总开关没关：{}", status["public_access"]));
            }
            Ok(String::new())
        })
        .await;

    report
        .run(
            "4 unpaired-chat",
            "没配对时直接发，应报对方服务器没配对",
            async {
                a.dispatch(Action::Chat {
                    action: ChatAction::DismissChatProblem,
                })?;
                a.dispatch(Action::Chat {
                    action: ChatAction::StartDirectChat {
                        user_id: probe_user.clone(),
                    },
                })?;
                let subscription = a.core.subscribe_chat();
                let problem = wait(a.name, "没配对的失败提示", || async {
                    let view = subscription.next().await?;
                    view.problem
                })
                .await
                .ok_or("120 秒没有失败提示")?;
                a.dispatch(Action::Chat {
                    action: ChatAction::DismissChatProblem,
                })?;
                let reason = reason_name(&problem.reason);
                if reason != "server_not_paired" {
                    return Err(format!("类别是 {reason}：{}", problem.message));
                }
                Ok(format!("类别 {reason}"))
            },
        )
        .await;

    report
        .run("5 public-on", "B 的公网总开关重新打开", async {
            let (status, bytes) = b
                .call(
                    Method::PUT,
                    "/_wonton/clients/public-access",
                    Some(&serde_json::json!({ "enabled": true })),
                )
                .await?;
            if status != StatusCode::OK {
                return Err(format!(
                    "PUT public-access 回 {status}：{}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
            Ok(String::new())
        })
        .await;

    report
        .run("6 peers-pair", "两台服务器配对", async {
            b.dispatch(Action::Peers {
                action: PeerAction::ShowPeerCode,
            })?;
            b.peers_until("出码", |view| view.code.clone().map(|_| ()))
                .await?;
            let code = b.peers().code.clone().expect("刚等到的码").code.clone();
            // iroh 的握手偶尔慢或第一次就失败：B 没收到请求就隔 10 秒再扫，重试算在同一个 WAIT 里。
            let deadline = Instant::now() + WAIT;
            loop {
                a.dispatch(Action::Peers {
                    action: PeerAction::ScanPeerCode { code: code.clone() },
                })?;
                let tried = tokio::time::timeout(
                    Duration::from_secs(10),
                    b.peers_until("收到配对请求", |view| {
                        (!view.requests.is_empty()).then_some(())
                    }),
                )
                .await;
                match tried {
                    Ok(result) => {
                        result?;
                        break;
                    }
                    Err(_) if Instant::now() < deadline => {}
                    Err(_) => {
                        return Err(format!("{} 120 秒没收到配对请求：{:#?}", b.name, b.peers()));
                    }
                }
            }
            // 同意以后也可能建不起连（iroh 握手第一下失败）：没连上就再扫一次码、再同意，仍在同一个 WAIT 里。
            let deadline = Instant::now() + WAIT;
            loop {
                if let Some(request) = b.peers().requests.first() {
                    b.dispatch(Action::Peers {
                        action: PeerAction::ApprovePeerRequest {
                            id: request.id.clone(),
                        },
                    })?;
                }
                let mut waiting = false;
                for (end, server) in [(&a, b_server.as_str()), (&b, a_server.as_str())] {
                    let tried = tokio::time::timeout(
                        Duration::from_secs(10),
                        end.peers_until("配对成立", |view| {
                            view.peers
                                .iter()
                                .any(|peer| {
                                    peer.server_name == server
                                        && peer.status == PeerStatus::Connected
                                })
                                .then_some(())
                        }),
                    )
                    .await;
                    match tried {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => return Err(error),
                        Err(_) => waiting = true,
                    }
                }
                if !waiting {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "120 秒没配对成立：A {:#?}\nB {:#?}",
                        a.peers(),
                        b.peers()
                    ));
                }
                a.dispatch(Action::Peers {
                    action: PeerAction::ScanPeerCode { code: code.clone() },
                })?;
            }
            Ok(String::new())
        })
        .await;

    report
        .run("7 friends", "B 加 A 为好友、A 同意", async {
            b.dispatch(Action::Friends {
                action: FriendsAction::AddFriend {
                    user_id: a_user.clone(),
                },
            })?;
            a.friends_until("收到好友请求", |view| {
                view.incoming
                    .iter()
                    .any(|request| request.user_id == b_user)
                    .then_some(())
            })
            .await?;
            let room = a
                .friends()
                .incoming
                .iter()
                .find(|request| request.user_id == b_user)
                .expect("刚等到的请求")
                .room_id
                .clone();
            a.dispatch(Action::Friends {
                action: FriendsAction::ApproveFriendRequest { room_id: room },
            })?;
            for (end, other) in [(&a, b_user.as_str()), (&b, a_user.as_str())] {
                end.friends_until("成为好友", |view| {
                    (view.state == FriendsState::Ready
                        && view.friends.iter().any(|friend| friend.user_id == other))
                    .then_some(())
                })
                .await?;
            }
            Ok(String::new())
        })
        .await;

    let mut timelines = None;
    report
        .run(
            "8 direct-chat-encrypted",
            "和对方发起私聊，房间开加密",
            async {
                a.dispatch(Action::Chat {
                    action: ChatAction::StartDirectChat {
                        user_id: b_user.clone(),
                    },
                })?;
                let subscription = a.core.subscribe_chat();
                let room = wait(a.name, "私聊房间出现", || async {
                    let view = subscription.next().await?;
                    view.rooms
                        .iter()
                        .find(|room| room.direct_user_id.as_deref() == Some(b_user.as_str()))
                        .map(|room| (room.room_id.clone(), room.is_encrypted))
                })
                .await
                .ok_or("120 秒没有私聊房间")?;
                if !room.1 {
                    return Err(format!("房间 {} 没开加密", room.0));
                }
                Ok(format!("房间 {}", room.0))
            },
        )
        .await;

    // 房间没出来就只能收工：后面两步都要它。
    let room = a
        .chat()
        .rooms
        .iter()
        .find(|room| room.direct_user_id.as_deref() == Some(b_user.as_str()))
        .map(|room| room.room_id.clone());
    let Some(room) = room else {
        report
            .run("9 send-decrypted", "A 发一句，B 出现同一句", async {
                Err("没有私聊房间".to_owned())
            })
            .await;
        report
            .run("10 offline-backfill", "停 B 的核心补发", async {
                Err("没有私聊房间".to_owned())
            })
            .await;
        finish(&report);
        return;
    };

    let first = "第一句：两台服务器之间的加密消息";
    report
        .run(
            "9 send-decrypted",
            "A 发一句，B 时间线出现同一句明文",
            async {
                let mut a_timeline = a.watch(&room).await?;
                let mut b_timeline = b.watch(&room).await?;
                a_timeline.send(first)?;
                a_timeline.until("A 的消息发出", &[first], true).await?;
                b_timeline.until("B 解出同一句", &[first], false).await?;
                timelines = Some((a_timeline, b_timeline));
                Ok(format!("B 看到：{first}"))
            },
        )
        .await;

    report
        .run(
            "10 offline-backfill",
            "B 的核心暂停时 A 又发三句，恢复后 120 秒内补到并解出",
            async {
                let (mut a_timeline, mut b_timeline) = match timelines {
                    Some(timelines) => timelines,
                    None => (a.watch(&room).await?, b.watch(&room).await?),
                };
                let offline = [
                    "第二句：离线期间的第一句",
                    "第三句：离线期间的第二句",
                    "第四句：离线期间的第三句",
                ];
                docker(&["pause", b_core.as_str()])?;
                let sent = async {
                    for body in offline {
                        a_timeline.send(body)?;
                    }
                    a_timeline.until("A 的三句都发出", &offline, true).await
                }
                .await;
                // 不管 A 发没发出去都放开 B，免得失败时留着一个冻住的核心。
                docker(&["unpause", b_core.as_str()])?;
                sent?;
                b_timeline.until("B 补齐三句", &offline, false).await?;
                Ok(format!("B 补齐：{offline:?}"))
            },
        )
        .await;

    finish(&report);
}

fn finish(report: &Report) {
    println!("RESULT: {} 步通过，{} 步失败", report.passed, report.failed);
    if report.failed > 0 {
        std::process::exit(1);
    }
}
