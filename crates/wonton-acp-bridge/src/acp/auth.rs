use std::{collections::HashMap, sync::Arc};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use super::{AcpClient, Inner, id_key, send_error, send_result};
use crate::error::{Error, Result};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl AuthMethod {
    pub fn is_terminal(&self) -> bool {
        self.kind.as_deref() == Some("terminal")
    }

    /// 桥只实现 terminal 与 agent（无 type）两种；env_var 等不认识的不显示。
    pub fn supported(&self) -> bool {
        matches!(
            self.kind.as_deref(),
            None | Some("agent") | Some("terminal")
        )
    }
}

pub struct UrlRequest {
    pub url: String,
    pub message: String,
    pub answer: oneshot::Sender<&'static str>,
}

pub struct AuthRequests {
    sender: mpsc::Sender<UrlRequest>,
    pub receiver: Mutex<mpsc::Receiver<UrlRequest>>,
    pending: Mutex<HashMap<String, oneshot::Sender<()>>>,
    active: std::sync::atomic::AtomicBool,
    notify: Notify,
}

impl AuthRequests {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            sender,
            receiver: Mutex::new(receiver),
            pending: Mutex::new(HashMap::new()),
            active: std::sync::atomic::AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn set_active(&self, active: bool) {
        self.active
            .store(active, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 丢弃上一次留下的请求，开始接收新的 URL 引导。
    async fn begin(&self) {
        self.set_active(true);
        let mut receiver = self.receiver.lock().await;
        while receiver.try_recv().is_ok() {}
        self.notify.notify_one();
    }

    async fn end(&self) {
        self.set_active(false);
        let mut receiver = self.receiver.lock().await;
        while receiver.try_recv().is_ok() {}
    }

    pub async fn cancel(&self, id: &Value) {
        if let Some(sender) = self.pending.lock().await.remove(&id_key(id)) {
            let _ = sender.send(());
        }
    }

    pub async fn close(&self) {
        self.pending.lock().await.clear();
    }
}

impl AcpClient {
    pub fn auth_methods(&self) -> &[AuthMethod] {
        &self.auth_methods
    }

    pub async fn next_login_url(&self) -> Option<UrlRequest> {
        // 不能持锁等：select! 会和 authenticate() 的 begin() 抢同一把锁而死锁。
        let auth = &self.inner.auth;
        loop {
            let notified = auth.notify.notified();
            {
                let mut receiver = auth.receiver.lock().await;
                match receiver.try_recv() {
                    Ok(request) => return Some(request),
                    Err(mpsc::error::TryRecvError::Disconnected) => return None,
                    Err(mpsc::error::TryRecvError::Empty) => {}
                }
            }
            notified.await;
        }
    }

    pub async fn authenticate(&self, method: &str) -> Result<()> {
        self.inner.auth.begin().await;
        let result = self
            .request("authenticate", json!({"methodId": method}))
            .await;
        self.inner.auth.end().await;
        // 登录错误可能含上游回显的授权码；只暴露错误类别和数字码。
        result.map_err(|error| Error::Upstream(format!("Agent 登录失败（ACP {}）", error.code)))?;
        Ok(())
    }
}

/// 登录方式逐项解析：一个坏项只跳过它自己，不让连接失败（向前兼容：不认识的项不显示）。
pub(super) fn parse_auth_methods(methods: Option<&Value>) -> Vec<AuthMethod> {
    let Some(methods) = methods.and_then(Value::as_array) else {
        return Vec::new();
    };
    methods
        .iter()
        .filter_map(
            |method| match serde_json::from_value::<AuthMethod>(method.clone()) {
                Ok(method) => Some(method),
                Err(error) => {
                    tracing::warn!(?error, method = %method, "跳过无法解析的 ACP 登录方式");
                    None
                }
            },
        )
        .collect()
}

pub(super) async fn elicit(inner: Arc<Inner>, id: Value, params: Value) {
    if params["mode"] != "url" {
        send_error(&inner, &id, -32602, "只支持 URL 登录引导").await;
        return;
    }
    let Some(url) = params["url"]
        .as_str()
        .and_then(|text| url::Url::parse(text).ok())
    else {
        send_error(&inner, &id, -32602, "登录 URL 无效").await;
        return;
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        send_error(&inner, &id, -32602, "登录 URL 必须是 HTTP 或 HTTPS").await;
        return;
    }
    if !inner.auth.is_active() {
        tracing::warn!(request = %id, "没有登录在进行，URL 引导立即回 cancel");
        send_result(&inner, &id, json!({"action": "cancel"})).await;
        return;
    }
    let (answer, result) = oneshot::channel();
    let (cancel, cancelled) = oneshot::channel();
    let key = id_key(&id);
    inner.auth.pending.lock().await.insert(key.clone(), cancel);
    let request = UrlRequest {
        url: url.to_string(),
        message: params["message"].as_str().unwrap_or_default().to_owned(),
        answer,
    };
    let sent = inner.auth.sender.try_send(request);
    if sent.is_ok() {
        inner.auth.notify.notify_one();
    }
    let action = if sent.is_ok() {
        tokio::select! {
            answer = result => answer.unwrap_or("cancel"),
            _ = cancelled => "cancel",
            _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => "cancel",
        }
    } else {
        "cancel"
    };
    inner.auth.pending.lock().await.remove(&key);
    send_result(&inner, &id, json!({"action": action})).await;
}
