//! 假核心记下的东西：排队的预设回应、按住的路径、收到的请求、引导状态和资料。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::Method;
use tokio::sync::watch;
use wonton_client::InstalledPlugin;

use super::answers::{Canned, Headered};
use super::bot_activity;
use super::router::owner_json;

/// 按住不回应的路径：测试要卡住一次请求时（如保存 key 途中再存一次），放行后才回。
type Holds = Mutex<HashMap<(Method, String), Arc<watch::Sender<bool>>>>;

/// 排队给一个方法+路径的带延迟预设回应。
type DelayedCanned = VecDeque<(Canned, Duration)>;

#[derive(Default)]
pub struct CoreState {
    /// 依次给出的已安装列表，最后一份一直用下去。
    pub(super) lists: Mutex<VecDeque<Vec<InstalledPlugin>>>,
    /// 假核心保存的引导状态和模型设置。
    pub(super) onboarding: Mutex<Option<serde_json::Value>>,
    /// 假核心保存的资料；为空时是还没填的。
    pub(super) owner: Mutex<Option<serde_json::Value>>,
    /// 某个方法和路径依次给出的回应：App 还会向核心要 Matrix 登录令牌，预设的回应不能让别的请求用掉。
    pub(super) canned: Mutex<HashMap<(Method, String), VecDeque<Canned>>>,
    /// 用完预设的回应后，某个方法和路径一直用的回应。
    pub(super) standing: Mutex<HashMap<(Method, String), Canned>>,
    /// 同上，但带响应头：有的接口把错误类别写在响应头里（如 `X-Wonton-Error`）。
    pub(super) canned_headered: Mutex<HashMap<(Method, String), Headered>>,
    pub(super) standing_headered: Mutex<HashMap<(Method, String), Headered>>,
    /// 按住不回应的路径：测试要卡住一次请求时（如保存 key 途中再存一次），放行后才回。
    pub(super) holds: Holds,
    /// 带延迟的预设回应：发出早的请求把它取走，睡完才回，制造“发得早、回得晚”。
    pub(super) canned_delayed: Mutex<HashMap<(Method, String), DelayedCanned>>,
    pub(super) seen: Mutex<Vec<Seen>>,
    pub family: Arc<crate::family_support::FamilyFake>,
    pub(super) bot_activity: bot_activity::Activity,
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: Method,
    pub path: String,
    pub language: Option<String>,
    pub from_page: bool,
    pub body: String,
}

impl CoreState {
    pub fn answer(&self, method: Method, path: &str, canned: Canned) {
        self.canned
            .lock()
            .unwrap()
            .entry((method, path.to_owned()))
            .or_default()
            .push_back(canned);
    }

    pub fn answer_always(&self, method: Method, path: &str, canned: Canned) {
        self.standing
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), canned);
    }

    pub fn answer_headered(&self, method: Method, path: &str, headered: Headered) {
        self.canned_headered
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), headered);
    }

    pub fn answer_always_headered(&self, method: Method, path: &str, headered: Headered) {
        self.standing_headered
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), headered);
    }

    pub fn stop_answering_headered(&self, method: Method, path: &str) {
        self.standing_headered
            .lock()
            .unwrap()
            .remove(&(method, path.to_owned()));
    }

    pub fn stop_answering(&self, method: Method, path: &str) {
        self.standing
            .lock()
            .unwrap()
            .remove(&(method, path.to_owned()));
    }

    /// 让这个请求一直等，直到 [`Self::release`]。
    pub fn hold(&self, method: Method, path: &str) -> Arc<watch::Sender<bool>> {
        let held = Arc::new(watch::channel(true).0);
        self.holds
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), held.clone());
        held
    }

    pub fn release(&self, method: Method, path: &str) {
        if let Some(held) = self.holds.lock().unwrap().get(&(method, path.to_owned())) {
            held.send_replace(false);
        }
    }

    /// 发得早的请求取走这份回应、睡完 `delay` 才回：制造“发得早、回得晚”的旧响应。
    pub fn answer_delayed(&self, method: Method, path: &str, canned: Canned, delay: Duration) {
        self.canned_delayed
            .lock()
            .unwrap()
            .entry((method, path.to_owned()))
            .or_default()
            .push_back((canned, delay));
    }

    pub fn set_lists(&self, lists: Vec<Vec<InstalledPlugin>>) {
        *self.lists.lock().unwrap() = lists.into();
    }

    pub fn set_onboarding(&self, onboarding: serde_json::Value) {
        *self.onboarding.lock().unwrap() = Some(onboarding);
    }

    pub fn set_owner(&self, name: Option<&str>, email: Option<&str>) {
        *self.owner.lock().unwrap() = Some(owner_json(name, email));
    }

    pub fn all_seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn seen(&self, method: &Method, path: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.method == *method && seen.path == path)
            .cloned()
            .collect()
    }
}
