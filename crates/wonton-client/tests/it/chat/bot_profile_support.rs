use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;
use wonton_matrix::MatrixSession;
use wonton_protocol::{BotActivityFacts, BotActivityReport};

pub(super) struct BotApi {
    report: watch::Sender<BotActivityReport>,
    pub requests: Mutex<Vec<Value>>,
    pub next_reply: Mutex<Option<(u16, Value)>>,
    pub events: Mutex<Vec<String>>,
    pub downloads: AtomicUsize,
    pub polls: AtomicUsize,
    pub broken: watch::Sender<bool>,
    target: Mutex<Option<(String, Arc<MatrixSession>)>>,
}

impl Default for BotApi {
    fn default() -> Self {
        Self {
            report: watch::Sender::new(BotActivityReport {
                rev: 0,
                bots: Vec::new(),
            }),
            requests: Mutex::default(),
            next_reply: Mutex::default(),
            events: Mutex::default(),
            downloads: AtomicUsize::default(),
            polls: AtomicUsize::default(),
            broken: watch::Sender::new(false),
            target: Mutex::default(),
        }
    }
}

impl BotApi {
    pub fn target(&self, url: &str, bot: Arc<MatrixSession>) {
        *self.target.lock().unwrap() = Some((url.to_owned(), bot));
    }

    pub fn facts(&self, facts: BotActivityFacts) {
        self.report.send_modify(|report| {
            report.rev += 1;
            report.bots = vec![facts];
        });
    }

    pub fn observe_matrix(&self, head: &str) {
        let line = head.lines().next().unwrap_or_default();
        // 只数媒体上传：加密的 `POST /keys/upload` 也带 “/upload”，SDK 在一次性密钥被领走后随时会补传。
        if line.starts_with("post ") && line.contains("/media/") && line.contains("/upload") {
            self.events.lock().unwrap().push("upload".to_owned());
        }
        if line.starts_with("get ") && line.contains("/download/") {
            self.downloads.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub async fn respond(&self, head: &str, body: &[u8]) -> Option<(u16, String)> {
        if head.starts_with("get /_wonton/api/bots/activity") {
            self.polls.fetch_add(1, Ordering::SeqCst);
            let mut report = self.report.subscribe();
            let mut broken = self.broken.subscribe();
            if *broken.borrow() {
                return Some((
                    503,
                    json!({"error":{"kind":"upstream","message":"Activity unavailable"}})
                        .to_string(),
                ));
            }
            let after = head
                .lines()
                .next()
                .unwrap()
                .split("?after=")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|rev| rev.parse::<u64>().ok());
            let current_rev = report.borrow_and_update().rev;
            if after == Some(current_rev) {
                tokio::select! {
                    _ = report.changed() => {},
                    _ = broken.changed() => {},
                    () = tokio::time::sleep(Duration::from_secs(25)) => {},
                }
            }
            if *broken.borrow() {
                return Some((503, "{}".to_owned()));
            }
            return Some((200, serde_json::to_string(&*report.borrow()).unwrap()));
        }
        if head.starts_with("put /_wonton/api/bots/") {
            let body: Value = serde_json::from_slice(body).unwrap();
            self.requests.lock().unwrap().push(body.clone());
            self.events.lock().unwrap().push("profile".to_owned());
            let reply = self.next_reply.lock().unwrap().take();
            if let Some((status, reply)) = reply {
                return Some((status, reply.to_string()));
            }
            let target = self.target.lock().unwrap().clone();
            if let Some((url, bot)) = target {
                if let Some(name) = body["name"].as_str() {
                    bot.set_display_name(name).await.unwrap();
                }
                if let Some(avatar) = body.get("avatar") {
                    let value = if avatar["action"] == "clear" {
                        json!("")
                    } else {
                        avatar["url"].clone()
                    };
                    set_field(&url, &bot, "avatar_url", json!({"avatar_url":value})).await;
                }
            }
            self.report.send_modify(|report| {
                report.rev += 1;
                for facts in &mut report.bots {
                    facts.profile_rev += 1;
                }
            });
            return Some((
                200,
                json!({"profile_rev":self.report.borrow().rev}).to_string(),
            ));
        }
        None
    }
}

pub(super) async fn set_field(url: &str, bot: &MatrixSession, key: &str, value: Value) {
    let saved = serde_json::to_value(bot.saved().unwrap()).unwrap();
    let response = reqwest::Client::new()
        .put(format!(
            "{url}/_matrix/client/v3/profile/{}/{key}",
            bot.user_id()
        ))
        .bearer_auth(saved["access_token"].as_str().unwrap())
        .header("content-type", "application/json")
        .body(value.to_string())
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "profile field failed: {}",
        response.text().await.unwrap()
    );
}

pub(super) async fn typing(url: &str, bot: &MatrixSession, room: &str, typing: bool) {
    let saved = serde_json::to_value(bot.saved().unwrap()).unwrap();
    let response = reqwest::Client::new()
        .put(format!(
            "{url}/_matrix/client/v3/rooms/{room}/typing/{}",
            bot.user_id()
        ))
        .bearer_auth(saved["access_token"].as_str().unwrap())
        .header("content-type", "application/json")
        .body(json!({"typing":typing,"timeout":30_000}).to_string())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}
