//! 跨机群：A 建群拉 B 的人和两边的 Bot，B 的人在群里点名 A 的 Bot 得回；B 整台停着时 A 在群里发的消息，
//! B 回来后补到。
//!
//! 两个人是真的 Matrix 账号（compose 里两台服务器的主人都叫 smokeowner，JWT 首次登录建号）；人是按“人”加联系人的，
//! Bot 发私聊邀请、人加入私聊接受。两个回声 Bot 在同一个
//! 房间里会互相回声停不下来，所以每一步只让一个 Bot 在线，谁在线由冒烟脚本停、起容器决定。

use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::json;
use std::time::Duration;

use super::bots::Bot;
use super::owner;

/// 一台服务器的主人的用户名（compose 里的 `WONTON_OWNER_NAME`）。
const HUMAN: &str = "smokeowner";
const A_HUMAN: &str = "@smokeowner:owner.test";
const B_HUMAN: &str = "@smokeowner:friend.test";
const A_BOT: &str = "@echo:owner.test";
const B_BOT: &str = "@echo:friend.test";

fn room_path() -> Result<std::path::PathBuf> {
    Ok(owner::state()?.join("group-room"))
}

/// 群 id 记在状态目录里给后面的步骤用：步骤之间只有它和测试机的磁盘能带东西过去。
fn remember(room: &str) -> Result<()> {
    Ok(std::fs::write(room_path()?, room)?)
}

fn room() -> Result<String> {
    let path = room_path()?;
    Ok(std::fs::read_to_string(&path)
        .with_context(|| {
            format!(
                "还没有群（{}），先跑 group-contacts、group-room",
                path.display()
            )
        })?
        .trim()
        .to_owned())
}

/// 两边的人和对方的 Bot 结成联系人：对方的 Bot 才理对方人的邀请和点名。人是按“人”加的（ADR-0050）：
/// Bot 拉起私聊邀请，对方接受（加入那个私聊）才算成立，成员状态由核心查出来。
pub async fn contacts() -> Result<()> {
    let (a, b) = super::owners().await?;
    // 先把两边的主人账号建出来：JWT 首次登录建号，邀请和入群都用它。
    let a_human = Bot::login("owner.test", HUMAN).await?;
    let b_human = Bot::login("friend.test", HUMAN).await?;
    let contacts = super::contacts_path();
    let (want_b, want_a) = tokio::try_join!(
        a.api(
            Method::POST,
            &contacts,
            Some(json!({"user_id": B_HUMAN, "kind": "person"})),
        ),
        b.api(
            Method::POST,
            &contacts,
            Some(json!({"user_id": A_HUMAN, "kind": "person"})),
        ),
    )?;
    super::show("B contacts", &want_b);
    super::show("A contacts", &want_a);
    accept(&b_human, B_HUMAN).await?;
    accept(&a_human, A_HUMAN).await?;
    let rows =
        super::wait_rows(&a, |rows| super::state_of(rows, B_HUMAN) == Some("active")).await?;
    super::show("A GET contacts", &rows);
    let rows =
        super::wait_rows(&b, |rows| super::state_of(rows, A_HUMAN) == Some("active")).await?;
    super::show("B GET contacts", &rows);
    a.close().await;
    b.close().await;
    Ok(())
}

/// `who` 接受发给自己的邀请房间。
async fn accept(who: &Bot, user: &str) -> Result<()> {
    for _ in 0..30 {
        let rooms = who.invites().await?;
        if !rooms.is_empty() {
            for room in rooms {
                who.join(&room).await?;
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("{user} 没收到联系人邀请")
}

/// A 的人建群，拉 B 的人、两边的 Bot；两边的 Bot 都接受邀请。
pub async fn create() -> Result<()> {
    let a = Bot::login("owner.test", HUMAN).await?;
    let id = a.group(&[B_HUMAN, A_BOT, B_BOT]).await?;
    remember(&id)?;
    member(&a, &id, A_BOT).await?;
    println!("{A_HUMAN} 建群 {id}，拉了 {B_HUMAN}、{A_BOT}、{B_BOT}");
    Ok(())
}

/// B 的人和 B 的 Bot 进群。
pub async fn join() -> Result<()> {
    let id = room()?;
    let b = Bot::login("friend.test", HUMAN).await?;
    b.join(&id).await?;
    member(&b, &id, B_BOT).await?;
    println!("{B_HUMAN} 和 {B_BOT} 进了 {id}");
    Ok(())
}

/// B 的人在群里点名 A 的 Bot：话跨机到 A，回声再跨机回来，B 的人收到。
pub async fn reply() -> Result<()> {
    let id = room()?;
    let b = Bot::login("friend.test", HUMAN).await?;
    let nonce = format!("n{}", super::bots::rand_suffix());
    let said = b.say(&id, A_BOT, &nonce).await?;
    println!("{B_HUMAN} 在群里：{said}");
    let reply = found(&b, &id, A_BOT, &nonce).await?;
    println!("{A_BOT} 在群里回：{reply}");
    Ok(())
}

/// B 整台停着时 A 的人在群里发一句，正文记下来给补发那一步对。
pub async fn send_while_offline() -> Result<()> {
    let id = room()?;
    let a = Bot::login("owner.test", HUMAN).await?;
    let text = format!("B 停着时的一句 n{}", super::bots::rand_suffix());
    a.send_text(&id, &text).await?;
    std::fs::write(owner::state()?.join("group-offline"), &text)?;
    println!("{A_HUMAN} 在 B 停着时发：{text}");
    Ok(())
}

/// B 回来后，那句补到了 B 这边。
pub async fn backfilled() -> Result<()> {
    let id = room()?;
    let text = std::fs::read_to_string(owner::state()?.join("group-offline"))?;
    let b = Bot::login("friend.test", HUMAN).await?;
    let got = found(&b, &id, A_HUMAN, text.trim()).await?;
    println!("{B_HUMAN} 收到补来的：{got}");
    Ok(())
}

/// 等 `user` 是房间成员。
async fn member(from: &Bot, room: &str, user: &str) -> Result<()> {
    for _ in 0..60 {
        if from.membership(room, user).await? == "join" {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("{user} 没进群")
}

/// 等房间里出现 `sender` 发的、带 `needle` 的消息：跨机补事件要时间。
async fn found(from: &Bot, room: &str, sender: &str, needle: &str) -> Result<String> {
    for _ in 0..150 {
        if let Some(body) = from.find(room, sender, needle).await? {
            return Ok(body);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    anyhow::bail!("{sender} 的 {needle} 一直没到")
}
