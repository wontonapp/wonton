//! 跨机 Bot 联系人（ADR-0050）的隔离端到端：两台真云吞经 iroh 配对，各装一个回声 Bot，走请求、同意、私聊、
//! 删除、拒绝和取消。用法见 deploy/dev/core/smoke-contacts.sh：每一步单独跑一次，步与步之间脚本停、起 Bot 的容器
//! （两个回声 Bot 在同一个房间里会互相回声停不下来，私聊时只留收消息的那一个在跑）。
#[path = "fr_contacts_smoke/bots.rs"]
mod bots;
#[path = "fr_share_smoke/owner.rs"]
mod owner;

use anyhow::{Context, Result, bail, ensure};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

use bots::Bot;
use owner::Owner;

const PLUGIN: &str = "agent.echo";
const A: &str = "@echo:owner.test";
const B: &str = "@echo:friend.test";

#[tokio::main]
async fn main() -> Result<()> {
    let step = std::env::args().nth(1).unwrap_or_default();
    match step.as_str() {
        "setup" => setup().await,
        "ignored" => ignored().await,
        "approve" => approve().await,
        "chat-a" => bots::chat("owner.test", "friend.test").await,
        "chat-b" => bots::chat("friend.test", "owner.test").await,
        "remove" => remove().await,
        "reject" => reject().await,
        "cancel" => cancel().await,
        _ => bail!(
            "usage: fr_contacts_smoke setup|ignored|approve|chat-a|chat-b|remove|reject|cancel"
        ),
    }
}

/// 打印一步的证据：接口和回应。
fn show(label: &str, value: &Value) {
    println!("{label}: {value}");
}

fn contacts_path() -> String {
    format!("/_wonton/api/bots/{PLUGIN}/default/contacts")
}

fn contact_path(user: &str) -> String {
    format!("{}/{user}", contacts_path())
}

async fn owners() -> Result<(Owner, Owner)> {
    Ok((Owner::open("owner").await?, Owner::open("friend").await?))
}

async fn setup() -> Result<()> {
    let (a, b) = owners().await?;
    owner::pair(&a, &b).await?;
    for (owner, name) in [(&a, "owner"), (&b, "friend")] {
        install(owner).await?;
        println!("installed {PLUGIN} on {name}.test");
    }
    let peers = a.api(Method::GET, "/_wonton/api/peers", None).await?;
    show("A GET /_wonton/api/peers", &peers["peers"]);
    a.close().await;
    b.close().await;
    Ok(())
}

async fn install(owner: &Owner) -> Result<()> {
    let mut tar = tar::Builder::new(Vec::new());
    tar.append_dir_all(".", "/plugins/echo-bot")?;
    let response = owner
        .http
        .post(format!("http://{}:8080/_wonton/api/plugins", owner.domain))
        .header("Content-Type", "application/x-tar")
        .timeout(Duration::from_secs(300))
        .body(tar.into_inner()?)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    ensure!(status.as_u16() == 201, "install {status}: {body}");
    Ok(())
}

/// 还不是联系人时，B 的 Bot 不接 A 的 Bot 的邀请。
async fn ignored() -> Result<()> {
    let from = Bot::login("owner.test", "echo").await?;
    let room = from.direct_room(B).await?;
    tokio::time::sleep(Duration::from_secs(20)).await;
    let membership = from.membership(&room, B).await?;
    println!("A bot invited B bot into {room}; after 20 s B bot membership = {membership}");
    ensure!(
        membership == "invite",
        "B bot answered a non-contact: {membership}"
    );
    from.leave(&room).await
}

async fn approve() -> Result<()> {
    let (a, b) = owners().await?;
    let request = json!({"user_id": B, "kind": "bot"});
    let sent = a.api(Method::POST, &contacts_path(), Some(request)).await?;
    show("A POST contacts", &sent);
    ensure!(
        sent["contacts"][0]["state"] == "pending",
        "request not pending"
    );
    let waiting = wait_confirmation(&b).await?;
    show(
        "B GET /_wonton/api/confirmations contact_requests",
        &waiting,
    );
    let approved = b
        .api(Method::POST, &format!("{}/approve", contact_path(A)), None)
        .await?;
    show("B POST approve", &approved);
    ensure!(approved["contacts"][0]["state"] == "active", "B not active");
    let rows = wait_rows(&a, |rows| state_of(rows, B) == Some("active")).await?;
    show("A GET contacts", &rows);
    a.close().await;
    b.close().await;
    Ok(())
}

async fn remove() -> Result<()> {
    let (a, b) = owners().await?;
    let removed = a.api(Method::DELETE, &contact_path(B), None).await?;
    show("A DELETE contact", &removed);
    ensure!(state_of(&removed, B).is_none(), "A still lists B");
    let rows = wait_rows(&b, |rows| state_of(rows, A).is_none()).await?;
    show("B GET contacts", &rows);
    a.close().await;
    b.close().await;
    Ok(())
}

async fn reject() -> Result<()> {
    let (a, b) = owners().await?;
    let request = json!({"user_id": B, "kind": "bot"});
    show(
        "A POST contacts",
        &a.api(Method::POST, &contacts_path(), Some(request)).await?,
    );
    show("B confirmations", &wait_confirmation(&b).await?);
    let rejected = b
        .api(Method::POST, &format!("{}/reject", contact_path(A)), None)
        .await?;
    show("B POST reject", &rejected);
    ensure!(state_of(&rejected, A).is_none(), "B still lists A");
    let rows = wait_rows(&a, |rows| state_of(rows, B) == Some("declined")).await?;
    show("A GET contacts", &rows);
    show(
        "A DELETE declined row",
        &a.api(Method::DELETE, &contact_path(B), None).await?,
    );
    a.close().await;
    b.close().await;
    Ok(())
}

async fn cancel() -> Result<()> {
    let (a, b) = owners().await?;
    let request = json!({"user_id": B, "kind": "bot"});
    show(
        "A POST contacts",
        &a.api(Method::POST, &contacts_path(), Some(request)).await?,
    );
    show("B confirmations", &wait_confirmation(&b).await?);
    show(
        "A DELETE pending (cancel)",
        &a.api(Method::DELETE, &contact_path(B), None).await?,
    );
    let rows = wait_rows(&b, |rows| state_of(rows, A).is_none()).await?;
    show("B GET contacts", &rows);
    let left = b
        .api(Method::GET, "/_wonton/api/confirmations", None)
        .await?;
    show("B confirmations after cancel", &left["contact_requests"]);
    ensure!(
        left["contact_requests"] == json!([]),
        "cancelled request still waits"
    );
    a.close().await;
    b.close().await;
    Ok(())
}

fn state_of<'a>(rows: &'a Value, user: &str) -> Option<&'a str> {
    rows["contacts"]
        .as_array()?
        .iter()
        .find(|row| row["user_id"] == user)
        .and_then(|row| row["state"].as_str())
}

/// 等对方核心收到经 Matrix 送来的请求。
async fn wait_confirmation(owner: &Owner) -> Result<Value> {
    for _ in 0..60 {
        let list = owner
            .api(Method::GET, "/_wonton/api/confirmations", None)
            .await?;
        let requests = &list["contact_requests"];
        if requests
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["contact"]["user_id"] == A))
        {
            return Ok(requests.clone());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!("contact request never reached the other keeper")
}

async fn wait_rows(owner: &Owner, done: impl Fn(&Value) -> bool) -> Result<Value> {
    let mut last = Value::Null;
    for _ in 0..60 {
        last = owner.api(Method::GET, &contacts_path(), None).await?;
        if done(&last) {
            return Ok(last);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(anyhow::anyhow!("contacts never settled: {last}")).context("wait_rows")
}
