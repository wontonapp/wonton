use serde_json::Value;
use std::sync::Mutex;
use tokio::sync::watch;

use super::{Core, Home};

pub(super) struct Starts {
    pub requests: Mutex<Vec<(String, Value)>>,
    held: watch::Sender<bool>,
    reply: Mutex<(u16, Value)>,
}

impl Default for Starts {
    fn default() -> Self {
        Self {
            requests: Mutex::default(),
            held: watch::Sender::new(false),
            reply: Mutex::new((
                500,
                serde_json::json!({"error": {"kind": "upstream", "message": "not configured"}}),
            )),
        }
    }
}

impl Home {
    pub fn hold_plugin_starts(&self, held: bool) {
        self.core.plugin_starts.held.send_replace(held);
    }

    pub fn answer_plugin_start(&self, status: u16, body: Value) {
        *self.core.plugin_starts.reply.lock().unwrap() = (status, body);
    }

    pub fn plugin_starts(&self) -> Vec<(String, Value)> {
        self.core.plugin_starts.requests.lock().unwrap().clone()
    }
}

pub(super) async fn respond(core: &Core, head: &str, body: &[u8]) -> Option<(u16, String)> {
    let rest = head.strip_prefix("post /_wonton/api/plugins/")?;
    let id = rest.split_whitespace().next()?.strip_suffix("/start")?;
    core.plugin_starts.requests.lock().unwrap().push((
        id.to_owned(),
        serde_json::from_slice(body).expect("start body"),
    ));
    let mut held = core.plugin_starts.held.subscribe();
    held.wait_for(|held| !held).await.unwrap();
    let (status, body) = core.plugin_starts.reply.lock().unwrap().clone();
    Some((status, body.to_string()))
}
