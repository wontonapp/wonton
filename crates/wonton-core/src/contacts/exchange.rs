//! 核心账号（`@wonton-core:<服务器名>`）之间的联系人房间：配对成立后按字节序较小的一方建房并邀请对方
//! （ADR-0050），两边只认已配对服务器的同类邀请；`app.wonton.contact` 事件经 `/sync` 收、按事务发。
//!
//! 发送是 outbox 模式：事件先和行一起落盘，发不出去留着，这里按每台服务器指数退避重试。建房不带邀请
//! （纯本机操作，不会因联邦不通卡住），房间 ID 落盘后单独 `/invite`，确认对方核心进房才发事件。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use wonton_protocol::{
    CONTACT_EVENT, CONTACT_ROOM_TYPE, CORE_LOCALPART, ContactError, ContactEvent, ContactOutcome,
};

use super::{Contacts, split_user};
use crate::matrix::MatrixError;

/// 长轮询一次的时长：核心账号的房间很少，长挂着比密集轮询省。
const SYNC_TIMEOUT: Duration = Duration::from_secs(25);

/// 同步循环出错后的重试间隔（传输抖动、Tuwunel 重启）。
const RETRY: Duration = Duration::from_secs(5);

/// `/sync` 的时间线只要联系人的事件类型：核心账号不进别的房间，邀请不受时间线过滤影响。
const SYNC_FILTER: &str = r#"{"room":{"timeline":{"types":[{"type":"app.wonton.contact"}]}}}"#;

/// `@wonton-core:<服务器名>`。
fn core_user_of(server: &str) -> String {
    format!("@{CORE_LOCALPART}:{server}")
}

/// 同步令牌存的位置：丢了就从全量同步重新来，处理是幂等的。
fn token_path(contacts: &Contacts) -> Option<PathBuf> {
    let parent = contacts.store_path().parent()?;
    Some(parent.join("contacts-sync-token"))
}

fn read_token(contacts: &Contacts) -> Option<String> {
    std::fs::read_to_string(token_path(contacts)?)
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

/// 收件循环的失败类型：Matrix 传输和联系人落盘分开，处理一样（记日志、稍后重试，令牌不推进）。
#[derive(Debug)]
enum PollError {
    Matrix(MatrixError),
    Contact(ContactError),
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollError::Matrix(error) => write!(f, "{error}"),
            PollError::Contact(error) => write!(f, "{}", error.as_str()),
        }
    }
}

/// 起 `/sync` 收件循环：邀请 → 加入已配对服务器的联系人房间；时间线 → `app.wonton.contact` 事件。
/// 每轮顺带把 outbox 里的事件按退避重发、补建自己该建的房间。没有 Matrix（有些开发模式不装 Tuwunel）时不起。
pub fn spawn(contacts: &Arc<Contacts>) {
    let Some(matrix) = contacts.matrix().cloned() else {
        return;
    };
    let contacts = contacts.clone();
    tokio::spawn(async move {
        let mut names = contacts.peers().paired_names();
        // 起来先对一遍名单：重启后补上漏掉的房间、把没发出去的事件发完。
        let paired = names.borrow_and_update().clone();
        maintain(&contacts, &paired).await;
        // outbox 的重发和房间补建不等 25s 的长轮询：2s 一拍。没活干时一拍只是一次锁内检查。
        tokio::spawn({
            let contacts = contacts.clone();
            let names = contacts.peers().paired_names();
            async move {
                let mut tick = tokio::time::interval(Duration::from_secs(2));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    let paired = names.borrow().clone();
                    maintain(&contacts, &paired).await;
                }
            }
        });
        let mut since = read_token(&contacts);
        loop {
            tokio::select! {
                _ = names.changed() => {}
                polled = tokio::spawn(poll_task(contacts.clone(), matrix.clone(), since.clone())) => {
                    match polled {
                        Ok(Ok(next)) => since = next,
                        Ok(Err(error)) => {
                            tracing::warn!(error = %error, "联系人收件同步出错，稍后重试");
                            tokio::time::sleep(RETRY).await;
                        }
                        Err(join_error) => {
                            tracing::warn!(error = %join_error, "联系人收件任务没跑完，稍后重试");
                            tokio::time::sleep(RETRY).await;
                        }
                    }
                }
            }
            let paired = names.borrow().clone();
            maintain(&contacts, &paired).await;
        }
    });
}

/// 单独一轮收件：`/sync` 一次、处理邀请和时间线、推进令牌。放在独立的任务里跑：配对名单变化打断
/// `select!` 时任务照常跑完（邀请、令牌都落了盘），不会停在“加了房没记下”的半截（取消安全）。
async fn poll_task(
    contacts: Arc<Contacts>,
    matrix: Arc<crate::matrix::Matrix>,
    since: Option<String>,
) -> Result<Option<String>, PollError> {
    let core = matrix.core_session();
    let response = core
        .sync(since.as_deref(), SYNC_TIMEOUT, Some(SYNC_FILTER))
        .await
        .map_err(PollError::Matrix)?;
    let next = response["next_batch"].as_str().map(str::to_owned);
    // 邀请：只接受已配对服务器核心账号发来的联系人房间，别的退掉。单个加入失败不挡整批，
    // 下次同步还会看到这条邀请。
    if let Some(invites) = response["rooms"]["invite"].as_object() {
        for (room, state) in invites {
            match invite_of_peer(&|server| contacts.peers().paired(server), state) {
                Some(server) => {
                    if let Err(error) = core.join(room).await {
                        tracing::warn!(peer = %server, room = %room, error = %error, "联系人房间加入失败，下次同步再试");
                        continue;
                    }
                    // 记房间是落盘操作：失败时整个同步作废（令牌不推进），下次重投，处理幂等。
                    contacts
                        .set_peer_room(&server, room)
                        .map_err(PollError::Contact)?;
                }
                None => {
                    let _ = core.leave(room).await;
                }
            }
        }
    }
    // 时间线：联系人房间里的 `app.wonton.contact`。任何一条落不了盘都作废整批，令牌不推进。
    if let Some(joined) = response["rooms"]["join"].as_object() {
        for (_room, body) in joined {
            let Some(events) = body["timeline"]["events"].as_array() else {
                continue;
            };
            for event in events {
                if event["type"].as_str() != Some(CONTACT_EVENT) {
                    continue;
                }
                let Some(sender) = event["sender"].as_str() else {
                    continue;
                };
                let Ok(parsed) = serde_json::from_value::<ContactEvent>(event["content"].clone())
                else {
                    continue;
                };
                apply(&contacts, sender, parsed)
                    .await
                    .map_err(PollError::Contact)?;
            }
        }
    }
    // 令牌在整批处理完才推进；写盘失败同样作废，下次从旧令牌重投。
    if let (Some(path), Some(token)) = (token_path(&contacts), next.clone()) {
        std::fs::write(path, token).map_err(|_| PollError::Contact(ContactError::Upstream))?;
    }
    Ok(next)
}

/// 每轮的维护：有活干的服务器（outbox 非空或房间还没建）才 drain，没活就是一次检查。
async fn maintain(contacts: &Arc<Contacts>, paired: &BTreeSet<String>) {
    for server in paired {
        let has_work =
            !contacts.lock().outbox_for(server).is_empty() || contacts.peer_room(server).is_none();
        if has_work {
            drain(contacts, server).await;
        }
    }
}

/// 给一台服务器送 outbox 里的事件：房间没建先建，对方没进房先邀请，都通了按序发；发成一条删一条。
/// 任何一步失败就停下，剩下的事件等下一轮（按服务器退避）。
pub(crate) async fn drain(contacts: &Arc<Contacts>, server: &str) {
    // 同一台服务器的发送全程串行：两个并发 drain 不会把同一事务发两遍。
    let _guard = contacts.exchange_lock().lock().await;
    if !contacts.room_ready(server) {
        return;
    }
    let room = match ensure_room(contacts, server).await {
        Ok(Some(room)) => room,
        // 我们是字节序较大的一方：等对方的建房邀请，不算失败。
        Ok(None) => return,
        Err(error) => {
            contacts.note_room_failure(server, error);
            return;
        }
    };
    let joined = match ensure_joined(contacts, server, &room).await {
        // 对方在房里（能发事件了），或邀请已在路上（等对方接受）：这一轮没有失败。
        Ok(joined) => {
            contacts.note_room_ok(server);
            joined
        }
        Err(error) => {
            contacts.note_room_failure(server, error);
            return;
        }
    };
    if !joined {
        // 对方还没进房：发了也看不见，事件留在 outbox 等下一轮。
        return;
    }
    let Some(matrix) = contacts.matrix() else {
        return;
    };
    let core = matrix.core_session();
    // 守卫不跨 await：先取快照再逐条发。
    let entries = contacts.lock().outbox_for(server);
    for entry in entries {
        let content = match serde_json::to_value(&entry.event) {
            Ok(content) => content,
            Err(_) => {
                contacts.note_room_failure(server, ContactError::Upstream);
                return;
            }
        };
        match core
            .send_event(&room, CONTACT_EVENT, &entry.txn, &content)
            .await
        {
            Ok(_) => {
                if let Err(error) = contacts.commit(|stored| stored.dequeue(&entry.txn)) {
                    contacts.note_room_failure(server, error);
                    return;
                }
            }
            Err(error) => {
                tracing::info!(peer = %server, error = %error, "联系人事件没发出去，稍后重试");
                contacts.note_room_failure(server, ContactError::PeerUnreachable);
                return;
            }
        }
    }
}

/// 把事件排进 outbox 并立刻试着发一轮；发不出去留在队列里，收件循环重试。
pub(crate) async fn queue_and_drain(
    contacts: &Arc<Contacts>,
    server: &str,
    event: ContactEvent,
    txn: String,
) {
    let at = crate::peers::now_ms();
    if contacts
        .commit(|stored| stored.enqueue(server, event, txn, at))
        .is_err()
    {
        return;
    }
    drain(contacts, server).await;
}

/// 和一台配对服务器的核心联系人房间没建起来时，按字节序较小的一方补建。建房不带邀请：纯本机操作，
/// 不因联邦不通卡住，也就不需要超时和超时后重建（会无限产生孤儿房间）。返回空表示我们是较大的一方，
/// 在等对方的邀请（被动，不算失败）。
async fn ensure_room(
    contacts: &Arc<Contacts>,
    server: &str,
) -> Result<Option<String>, ContactError> {
    if let Some(room) = contacts.peer_room(server) {
        return Ok(Some(room));
    }
    let Some(matrix) = contacts.matrix() else {
        return Err(ContactError::Upstream);
    };
    if matrix.server_name() >= server {
        return Ok(None);
    }
    // 调用方（drain）已经串行化同一台服务器的操作，这里不用再拿交换锁。
    if let Some(room) = contacts.peer_room(server) {
        return Ok(Some(room));
    }
    let core = matrix.core_session();
    let room = core
        .create_contact_room_no_invite()
        .await
        .map_err(|error| {
            tracing::warn!(peer = %server, error = %error, "联系人房间建不起来");
            ContactError::PeerUnreachable
        })?;
    contacts.set_peer_room(server, &room)?;
    tracing::info!(peer = %server, room = %room, "联系人房间已建立");
    Ok(Some(room))
}

/// 确认对方的核心账号进了联系人房间：不在就补一次邀请（createRoom 里那次邀请会撞上联邦没就绪）。
/// 返回要不要现在就发事件（对方在房里才算）。重试由调用方按退避管。
async fn ensure_joined(
    contacts: &Arc<Contacts>,
    server: &str,
    room: &str,
) -> Result<bool, ContactError> {
    let matrix = contacts.matrix().ok_or(ContactError::Upstream)?;
    let core = matrix.core_session();
    let peer_core = core_user_of(server);
    if room_contains(contacts, room, &peer_core).await? {
        return Ok(true);
    }
    // 已在邀请里、对方还没接受时 Tuwunel 会拒绝重发（M_FORBIDDEN）：邀请已经在路上，等对方接受，
    // 不算这轮失败。别的错误（传输、账号问题）如实上报。
    match core.invite(room, &peer_core).await {
        Ok(()) => {
            tracing::info!(peer = %server, room = %room, "邀请已发出，等对方核心进房");
            Ok(false)
        }
        Err(error) if error.errcode() == "M_FORBIDDEN" => {
            tracing::debug!(peer = %server, room = %room, "邀请已在路上，等对方核心进房");
            Ok(false)
        }
        Err(error) => {
            tracing::debug!(peer = %server, room = %room, error = %error, "邀请没发出去");
            Err(ContactError::PeerUnreachable)
        }
    }
}

/// 邀请的对方服务器名：房间类型对上、邀请人是 `@wonton-core:<已配对的服务器>` 时返回那台的名字。
fn invite_of_peer(paired: &dyn Fn(&str) -> bool, state: &Value) -> Option<String> {
    let events = state["invite_state"]["events"].as_array()?;
    let mut create_typed = false;
    let mut inviter = None;
    for event in events {
        match event["type"].as_str() {
            Some("m.room.create") => {
                create_typed = event["content"]["type"].as_str() == Some(CONTACT_ROOM_TYPE);
            }
            Some("m.room.member") if event["content"]["membership"].as_str() == Some("invite") => {
                inviter = event["sender"].as_str().map(str::to_owned);
            }
            _ => {}
        }
    }
    let inviter = inviter?;
    let (_, server) = split_user(&inviter)?;
    (create_typed && core_user_of(&server) == inviter && paired(&server)).then_some(server)
}

/// 三方核对通过吗（`check_parties` 的布尔版，给不返回服务器名的分支）。
fn parties_match(contacts: &Contacts, sender: &str, from: &str, to: &str) -> bool {
    let Some(own) = contacts.matrix().map(|m| m.server_name().to_owned()) else {
        return false;
    };
    let paired = |server: &str| contacts.peers().paired(server);
    check_parties(&own, &paired, sender, from, to).is_some()
}

/// 一条收到的 `app.wonton.contact` 事件：核对三方后落到状态机。回复事件进 outbox。
/// 返回有没有改动（调试用）；落盘失败原样上抛，整批同步作废。
async fn apply(
    contacts: &Arc<Contacts>,
    sender: &str,
    event: ContactEvent,
) -> Result<bool, ContactError> {
    match event {
        ContactEvent::Request {
            id,
            from,
            to,
            from_display_name,
            keeper_display_name,
        } => {
            let Some(own) = contacts.matrix().map(|m| m.server_name().to_owned()) else {
                return Ok(false);
            };
            let paired = |server: &str| contacts.peers().paired(server);
            let Some(from_server) = check_parties(&own, &paired, sender, &from, &to) else {
                return Ok(false);
            };
            // 请求的收件 Bot 得是本机 Bot（`to` 认）：不是的时候回“没有这个人”（协议 ContactOutcome::NotFound）。
            let Some(bots) = contacts.bots() else {
                return Ok(false);
            };
            let Some(bot) = bots.bot_of_user(&to) else {
                queue_and_drain(
                    contacts,
                    &from_server,
                    ContactEvent::Result {
                        id,
                        from: to,
                        to: from,
                        outcome: ContactOutcome::NotFound,
                        keeper_display_name: None,
                    },
                    crate::secret::random_token(),
                )
                .await;
                return Ok(false);
            };
            let reply = contacts.on_remote_request(
                &bot,
                &from,
                &from_server,
                &id,
                from_display_name,
                keeper_display_name,
            )?;
            if let Some(reply) = reply {
                queue_and_drain(contacts, &from_server, reply, id).await;
            }
            Ok(true)
        }
        ContactEvent::Result {
            id,
            from,
            to,
            outcome,
            keeper_display_name,
        } => {
            if !parties_match(contacts, sender, &from, &to) {
                return Ok(false);
            }
            // `to` 认定本机 Bot；对不上（没有这个 Bot）整条忽略。
            let Some(bots) = contacts.bots() else {
                return Ok(false);
            };
            let Some(bot) = bots.bot_of_user(&to) else {
                return Ok(false);
            };
            let from_server = split_user(&from)
                .map(|(_, server)| server)
                .unwrap_or_default();
            contacts.on_remote_result(
                &bot,
                &from,
                &from_server,
                &id,
                outcome,
                keeper_display_name,
            )?;
            Ok(true)
        }
        ContactEvent::Remove { from, to } => {
            if !parties_match(contacts, sender, &from, &to) {
                return Ok(false);
            }
            let Some(bots) = contacts.bots() else {
                return Ok(false);
            };
            let Some(bot) = bots.bot_of_user(&to) else {
                return Ok(false);
            };
            let from_server = split_user(&from)
                .map(|(_, server)| server)
                .unwrap_or_default();
            contacts.on_remote_remove(&bot, &from, &from_server)?;
            Ok(true)
        }
        // 比这台核心新的核心发的事件，忽略。
        ContactEvent::Other => Ok(false),
    }
}

/// 事件三方的核对（ADR-0050）：发送者是 `from` 所在服务器的核心账号，那台已配对；`to` 在本机。
/// 通过时返回 `from` 的服务器名。核对逻辑是纯函数，`paired` 由调用方从配对表来，测试好造假。
fn check_parties(
    own: &str,
    paired: &dyn Fn(&str) -> bool,
    sender: &str,
    from: &str,
    to: &str,
) -> Option<String> {
    let (_, sender_server) = split_user(sender)?;
    let (_, from_server) = split_user(from)?;
    let (_, to_server) = split_user(to)?;
    (sender_server == from_server
        && to_server == own
        && core_user_of(&from_server) == sender
        && paired(&from_server))
    .then(|| from_server.to_owned())
}

/// 归属人请远端的人做联系人：Bot 发带 `app.wonton.bot_contact` 的私聊邀请（ADR-0050）。
/// 邀请被拒（MSC4155）落 Declined、查无此人落 NotFound，连接问题不记（协议 ContactError 的约定）。
pub(crate) enum Invited {
    /// 邀请发出去了，对方还没表态。
    Sent(String),
    /// 对方拒绝了这个 Bot（或这台服务器）：记录这行，壳显示“对方拒绝了”。
    Declined,
    /// 对方服务器上没有这个人：记录这行，壳显示“没有这个人”。
    Missing,
}

pub(crate) async fn invite_person(
    contacts: &Arc<Contacts>,
    bot: &wonton_protocol::BotRef,
    user: &str,
    server: &str,
) -> Result<Invited, ContactError> {
    let matrix = contacts.matrix().ok_or(ContactError::Upstream)?;
    let bots = contacts.bots().ok_or(ContactError::Upstream)?;
    if !contacts.peers().paired(server) {
        return Err(ContactError::NotPaired);
    }
    let Some(localpart) = bots.bot_localpart(bot) else {
        return Err(ContactError::BotNotFound);
    };
    // 以 Bot 的身份登录一台一次性设备发邀请，发完就退。
    let login = matrix
        .login(&localpart, "wonton-contacts-invite")
        .await
        .map_err(invite_error)?;
    let session = matrix.session(&login);
    let room = session.create_bot_contact_room(user).await;
    let _ = session.logout().await;
    match room {
        Ok(room) => Ok(Invited::Sent(room)),
        Err(error) => match error.errcode() {
            "M_FORBIDDEN" => Ok(Invited::Declined),
            "M_NOT_FOUND" => Ok(Invited::Missing),
            _ => Err(invite_error(error)),
        },
    }
}

/// 撤回对远端的人的邀请：踢出并退房（照 `Session::abandon`）。
pub(crate) async fn revoke_person_invite(
    contacts: &Arc<Contacts>,
    bot: &wonton_protocol::BotRef,
    room: &str,
    user: &str,
) {
    let Some(matrix) = contacts.matrix().cloned() else {
        return;
    };
    let Some(bots) = contacts.bots().cloned() else {
        return;
    };
    let Some(localpart) = bots.bot_localpart(bot) else {
        return;
    };
    let Ok(login) = matrix.login(&localpart, "wonton-contacts-invite").await else {
        tracing::warn!(bot = %bot.bot, "撤回邀请时登不进 Bot 的账号");
        return;
    };
    let session = matrix.session(&login);
    let langs = crate::plugin::languages::Languages::from_header(None);
    let _ = session.abandon(room, user, &langs).await;
    let _ = session.logout().await;
}

/// 远端的人接没接受邀请：查私聊房间的成员列表（Tuwunel 管理接口，2026-11-12 对 v1.9.4 实测开放）。
pub(crate) async fn room_contains(
    contacts: &Arc<Contacts>,
    room: &str,
    user: &str,
) -> Result<bool, ContactError> {
    let matrix = contacts.matrix().ok_or(ContactError::Upstream)?;
    let core = matrix.core_session();
    let members = core
        .room_members(room)
        .await
        .map_err(|_| ContactError::Upstream)?;
    Ok(members.iter().any(|member| member == user))
}

/// 邀请被拒（M_FORBIDDEN，MSC4155）和查无此人（M_NOT_FOUND）在 `invite_person` 里落成行的状态，不算错误；
/// 登录本身失败只剩传输和上游的问题。
fn invite_error(_: MatrixError) -> ContactError {
    ContactError::PeerUnreachable
}
