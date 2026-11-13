//! 冒烟里扮演 Bot：用测试的 JWT 签发密钥登录 Bot 的账号（新设备），经各自服务器的 TLS 前门说 Matrix。房间不加密：
//! 回声 Bot 两种都回，冒烟只看名单放不放行。

use anyhow::{Context, Result, ensure};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::time::Duration;

pub struct Bot {
    http: Client,
    server: String,
    pub user_id: String,
    token: String,
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

impl Bot {
    pub async fn login(server: &str, localpart: &str) -> Result<Self> {
        let http = super::owner::https()?;
        let key = std::fs::read_to_string(super::owner::state()?.join("matrix.key"))?;
        let issuer =
            wonton_core::matrix::issuer::Issuer::from_pem(&key).map_err(anyhow::Error::msg)?;
        let login: Value = http
            .post(format!("https://{server}/_matrix/client/v3/login"))
            .json(&json!({"type": "org.matrix.login.jwt", "token": issuer.login_token(localpart)}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(Self {
            http,
            server: server.to_owned(),
            user_id: login["user_id"].as_str().context("user id")?.to_owned(),
            token: login["access_token"].as_str().context("token")?.to_owned(),
        })
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("https://{}{path}", self.server))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        ensure!(status.is_success(), "{path}: {status} {text}");
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// 建一个不加密的私聊并邀请对方。邀请单独发：建房时顺带的邀请失败了只记服务器日志；刚配对时联邦要一会儿才通，
    /// 失败了隔几秒再发。
    pub async fn direct_room(&self, invitee: &str) -> Result<String> {
        let body = json!({"preset": "private_chat", "is_direct": true});
        let room = self
            .call(Method::POST, "/_matrix/client/v3/createRoom", Some(body))
            .await?;
        let room = room["room_id"].as_str().context("room id")?.to_owned();
        self.invite(&room, invitee).await?;
        Ok(room)
    }

    /// 邀请一个用户，联邦没通时隔几秒再发。
    pub async fn invite(&self, room: &str, user: &str) -> Result<()> {
        let path = format!("/_matrix/client/v3/rooms/{}/invite", encode(room));
        let mut last = None;
        for _ in 0..12 {
            match self
                .call(Method::POST, &path, Some(json!({"user_id": user})))
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => last = Some(e),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        Err(last.context("invite")?)
    }

    /// 建一个不加密的群，把 `invitees` 一次拉进来。
    pub async fn group(&self, invitees: &[&str]) -> Result<String> {
        let body = json!({"preset": "private_chat", "is_direct": false});
        let room = self
            .call(Method::POST, "/_matrix/client/v3/createRoom", Some(body))
            .await?;
        let room = room["room_id"].as_str().context("room id")?.to_owned();
        for invitee in invitees {
            self.invite(&room, invitee).await?;
        }
        Ok(room)
    }

    /// 接受房间邀请。
    pub async fn join(&self, room: &str) -> Result<()> {
        let path = format!("/_matrix/client/v3/rooms/{}/join", encode(room));
        self.call(Method::POST, &path, Some(json!({}))).await?;
        Ok(())
    }

    /// 还没接受的邀请房间。
    pub async fn invites(&self) -> Result<Vec<String>> {
        let sync: Value = self
            .call(Method::GET, "/_matrix/client/v3/sync?timeout=0", None)
            .await?;
        Ok(sync["rooms"]["invite"]
            .as_object()
            .map(|rooms| rooms.keys().cloned().collect())
            .unwrap_or_default())
    }

    pub async fn membership(&self, room: &str, user: &str) -> Result<String> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/state/m.room.member/{}",
            encode(room),
            encode(user)
        );
        let member = self.call(Method::GET, &path, None).await?;
        Ok(member["membership"].as_str().unwrap_or("none").to_owned())
    }

    /// 点名对方说一句，返回发出的正文。
    pub async fn say(&self, room: &str, to: &str, nonce: &str) -> Result<String> {
        let body = format!("{to} 你好，我是 {} {nonce}", self.user_id);
        let content = json!({"msgtype": "m.text", "body": body, "m.mentions": {"user_ids": [to]}});
        self.put(room, content, nonce).await?;
        Ok(body)
    }

    /// 往房间发一条纯文字。
    pub async fn send_text(&self, room: &str, body: &str) -> Result<()> {
        let content = json!({"msgtype": "m.text", "body": body});
        self.put(room, content, &rand_suffix().to_string()).await
    }

    async fn put(&self, room: &str, content: Value, txn: &str) -> Result<()> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/send/m.room.message/{txn}",
            encode(room)
        );
        self.call(Method::PUT, &path, Some(content)).await?;
        Ok(())
    }

    /// 房间里 `sender` 最近发的一条正文里带 `needle` 的消息。
    pub async fn find(&self, room: &str, sender: &str, needle: &str) -> Result<Option<String>> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/messages?dir=b&limit=30",
            encode(room)
        );
        let page = self.call(Method::GET, &path, None).await?;
        Ok(page["chunk"].as_array().and_then(|events| {
            events
                .iter()
                .filter(|event| event["type"] == "m.room.message" && event["sender"] == sender)
                .filter_map(|event| event["content"]["body"].as_str())
                .find(|body| body.contains(needle))
                .map(str::to_owned)
        }))
    }

    pub async fn leave(&self, room: &str) -> Result<()> {
        let path = format!("/_matrix/client/v3/rooms/{}/leave", encode(room));
        self.call(Method::POST, &path, Some(json!({}))).await?;
        Ok(())
    }
}

/// `from` 服务器上的回声 Bot 停着时，测试扮演它：建私聊邀请对方的回声 Bot，等它进来，点名说一句，等它回声。
/// 说完退出房间：它的进程再起来时不接着回声，不然两边的回声 Bot 会在这个房间里互相回个没完。
pub async fn chat(from: &str, to: &str) -> Result<()> {
    let speaker = Bot::login(from, "echo").await?;
    let listener = format!("@echo:{to}");
    let room = speaker.direct_room(&listener).await?;
    let mut joined = false;
    for _ in 0..60 {
        if speaker.membership(&room, &listener).await? == "join" {
            joined = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    ensure!(
        joined,
        "{listener} did not accept the invite from its contact"
    );
    println!("{} invited {listener} into {room}: joined", speaker.user_id);
    let nonce = format!("n{}", rand_suffix());
    let said = speaker.say(&room, &listener, &nonce).await?;
    println!("{} -> {listener}: {said}", speaker.user_id);
    let mut reply = None;
    for _ in 0..60 {
        reply = speaker.find(&room, &listener, &nonce).await?;
        if reply.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let reply = reply.with_context(|| format!("{listener} never answered"))?;
    println!("{listener} -> {}: {reply}", speaker.user_id);
    speaker.leave(&room).await
}

pub(super) fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64)
}
