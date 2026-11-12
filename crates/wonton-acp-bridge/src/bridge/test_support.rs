//! 桥的行为测试用的替身：内存房间记录所有 Matrix 侧副作用，假卡片接口收下确认请求
//! 等测试回答，ACP 侧用 `acp::test_support::FakeAgent` 的 Manual 场景驱动。

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::{Stream, StreamExt, stream};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use wonton_matrix::{Message, MessageBody, OwnedUserId, SendStatus};

use crate::error::Result;
use crate::room::Room;

/// 记录下来的对外副作用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    MainText(String),
    ThreadText {
        root: String,
        text: String,
    },
    Notice(String),
    ThreadMention {
        root: String,
        text: String,
    },
    ThreadNotice {
        root: String,
        text: String,
    },
    Typing(bool),
    Redact(String),
    State(String),
    /// 一条 Agent 进度状态消息（ADR-0082），内容原样。
    RunStatus {
        event_id: String,
        thread: Option<String>,
        content: Value,
    },
    /// 对状态消息的编辑（`m.replace`），内容原样。
    ReplaceRunStatus {
        event_id: String,
        content: Value,
    },
}

#[derive(Default)]
struct Recorded {
    effects: Vec<Effect>,
    roots: u64,
    statuses: u64,
    updates: Option<mpsc::UnboundedSender<Vec<Message>>>,
}

/// 内存房间：`updates` 流由测试 push，发送只记录。
pub struct FakeRoom {
    id: String,
    /// 首屏快照：和真时间线一样，updates 流的第一项是订阅时刻已有的消息（重启后停机
    /// 期间发来的都在里面）。
    snapshot: Vec<Message>,
    recorded: Arc<Mutex<Recorded>>,
    updates: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<Message>>>>,
    /// 可下载的媒体：句柄 → 字节。没登记的句柄按下载失败处理。
    media: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl FakeRoom {
    /// 首屏快照：和真时间线一样，updates 流的第一项是订阅时刻已有的消息（重启后停机
    /// 期间发来的都在里面）；全新房间传空列表。
    pub fn with_snapshot(id: &str, snapshot: Vec<Message>) -> Self {
        let (updates, receiver) = mpsc::unbounded_channel();
        let recorded = Arc::new(Mutex::new(Recorded {
            effects: Vec::new(),
            roots: 0,
            statuses: 0,
            updates: Some(updates),
        }));
        Self {
            id: id.to_owned(),
            snapshot,
            recorded,
            updates: Arc::new(tokio::sync::Mutex::new(receiver)),
            media: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 登记一个可下载的媒体句柄。
    pub fn serve_media(&self, token: &str, bytes: Vec<u8>) {
        self.media.lock().unwrap().insert(token.to_owned(), bytes);
    }

    pub fn push(&self, messages: Vec<Message>) {
        if let Some(updates) = &self.recorded.lock().unwrap().updates {
            let _ = updates.send(messages);
        }
    }

    /// 关掉消息流（丢掉发送端，接收端读完返回 None）。不碰被 recv 借着的接收锁。
    pub fn close(&self) {
        self.recorded.lock().unwrap().updates = None;
    }

    pub fn effects(&self) -> Vec<Effect> {
        self.recorded.lock().unwrap().effects.clone()
    }

    pub fn texts(&self) -> Vec<String> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::MainText(text) | Effect::Notice(text) => Some(text),
                Effect::ThreadText { text, .. }
                | Effect::ThreadMention { text, .. }
                | Effect::ThreadNotice { text, .. } => Some(text),
                Effect::Typing(_)
                | Effect::Redact(_)
                | Effect::State(_)
                | Effect::RunStatus { .. }
                | Effect::ReplaceRunStatus { .. } => None,
            })
            .collect()
    }

    /// 主对话里发出的文字（不含 notice）。
    pub fn main_texts(&self) -> Vec<String> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::MainText(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    /// 开过的话题根（按出现顺序）。
    pub fn roots(&self) -> Vec<String> {
        let count = self.recorded.lock().unwrap().roots;
        (1..=count).map(|index| format!("$root{index}")).collect()
    }

    /// 发出的状态消息（事件 ID、话题根、完整内容），按发送顺序。
    pub fn run_statuses(&self) -> Vec<(String, Option<String>, Value)> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::RunStatus {
                    event_id,
                    thread,
                    content,
                } => Some((event_id, thread, content)),
                _ => None,
            })
            .collect()
    }

    /// 对某条状态消息的全部编辑内容（`m.replace`），按顺序。
    pub fn run_edits(&self, event_id: &str) -> Vec<Value> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::ReplaceRunStatus {
                    event_id: target,
                    content,
                } if target == event_id => Some(content),
                _ => None,
            })
            .collect()
    }

    pub fn thread_texts(&self, root: &str) -> Vec<String> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::ThreadText { root: event, text } if event == root => Some(text),
                _ => None,
            })
            .collect()
    }

    fn record(&self, effect: Effect) {
        self.recorded.lock().unwrap().effects.push(effect);
    }
}

fn text(state: &mut Recorded, effect: &str, body: &str) {
    let effect = match effect {
        "thread" => Effect::ThreadText {
            root: String::new(),
            text: body.to_owned(),
        },
        "notice" => Effect::Notice(body.to_owned()),
        _ => Effect::MainText(body.to_owned()),
    };
    state.effects.push(effect);
}

impl Room for FakeRoom {
    fn room_id(&self) -> String {
        self.id.clone()
    }

    async fn send_text(&self, body: &str) -> Result<()> {
        let mut state = self.recorded.lock().unwrap();
        text(&mut state, "main", body);
        Ok(())
    }

    async fn send_text_event(&self, body: &str) -> Result<String> {
        let mut state = self.recorded.lock().unwrap();
        text(&mut state, "main", body);
        state.roots += 1;
        Ok(format!("$root{}", state.roots))
    }

    async fn send_thread_mention(
        &self,
        root: &str,
        body: &str,
        _owner: &wonton_matrix::OwnedUserId,
    ) -> Result<()> {
        let mut state = self.recorded.lock().unwrap();
        state.effects.push(Effect::ThreadMention {
            root: root.to_owned(),
            text: body.to_owned(),
        });
        Ok(())
    }

    async fn send_thread_notice(&self, root: &str, body: &str) -> Result<()> {
        let mut state = self.recorded.lock().unwrap();
        state.effects.push(Effect::ThreadNotice {
            root: root.to_owned(),
            text: body.to_owned(),
        });
        Ok(())
    }

    async fn send_thread_text(&self, root: &str, body: &str) -> Result<()> {
        let mut state = self.recorded.lock().unwrap();
        state.effects.push(Effect::ThreadText {
            root: root.to_owned(),
            text: body.to_owned(),
        });
        Ok(())
    }

    async fn send_notice(&self, body: &str) -> Result<()> {
        let mut state = self.recorded.lock().unwrap();
        text(&mut state, "notice", body);
        Ok(())
    }

    async fn send_run_status(&self, content: Value) -> Result<String> {
        let mut state = self.recorded.lock().unwrap();
        state.statuses += 1;
        let event_id = format!("$status{}", state.statuses);
        let thread = content["m.relates_to"]["event_id"]
            .as_str()
            .map(str::to_owned);
        state.effects.push(Effect::RunStatus {
            event_id: event_id.clone(),
            thread,
            content,
        });
        Ok(event_id)
    }

    async fn replace_run_status(&self, content: Value) -> Result<()> {
        let mut state = self.recorded.lock().unwrap();
        let event_id = content["m.relates_to"]["event_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        state
            .effects
            .push(Effect::ReplaceRunStatus { event_id, content });
        Ok(())
    }

    async fn typing(&self, typing: bool) -> Result<()> {
        self.record(Effect::Typing(typing));
        Ok(())
    }

    async fn redact(&self, event_id: &str, _reason: &str) -> Result<()> {
        self.record(Effect::Redact(event_id.to_owned()));
        Ok(())
    }

    async fn download_media(&self, token: &str) -> Result<Vec<u8>> {
        self.media
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .ok_or_else(|| {
                crate::error::Error::Matrix(crate::error::Detail::data(format!(
                    "M_NOT_FOUND: {token}"
                )))
            })
    }

    async fn set_state(&self, event_type: &str, _state_key: &str, _content: Value) -> Result<()> {
        self.record(Effect::State(event_type.to_owned()));
        Ok(())
    }

    async fn updates(&self) -> impl Stream<Item = Vec<Message>> + Unpin + Send {
        let snapshot = self.snapshot.clone();
        let updates = self.updates.clone();
        // 和真实时间线一样，第一项是当前的消息（重启后的快照，全新房间是空列表）。
        Box::pin(
            stream::once(async { snapshot })
                .chain(stream::unfold(updates, |updates| async move {
                    let mut receiver = updates.lock().await;
                    let messages = receiver.recv().await;
                    drop(receiver);
                    messages.map(|messages| (messages, updates))
                }))
                .fuse(),
        )
    }
}

/// 假的核心确认接口：收下请求等测试回答。
pub struct FakeCards {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>>,
    task: JoinHandle<()>,
}

impl FakeCards {
    pub async fn start() -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(VecDeque::new()));
        let task = tokio::spawn(serve_cards(listener, requests.clone(), answers.clone()));
        Self {
            url: format!("http://{address}/mcp"),
            requests,
            answers,
            task,
        }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }

    /// 回答最近一个还没答的卡片。
    pub fn answer(&self, choice_id: &str) {
        self.answer_raw(serde_json::json!({"outcome": "selected", "choice_id": choice_id}));
    }

    pub fn answer_raw(&self, body: Value) {
        let sender = self.answers.lock().unwrap().pop_front();
        if let Some(sender) = sender {
            let _ = sender.send(body);
        }
    }

    pub fn pending(&self) -> usize {
        self.answers.lock().unwrap().len()
    }

    pub fn finish(&self) {
        self.task.abort();
    }
}

async fn serve_cards(
    listener: TcpListener,
    requests: Arc<Mutex<Vec<Value>>>,
    answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>>,
) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let requests = requests.clone();
        let answers = answers.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(&mut stream);
            let mut content_length = 0;
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).await.is_err() {
                return;
            }
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.is_err() {
                    return;
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
            if let Ok(request) = serde_json::from_slice::<Value>(&body) {
                requests.lock().unwrap().push(request);
            }
            let (sender, receiver) = oneshot::channel();
            answers.lock().unwrap().push_back(sender);
            let Ok(answer) = receiver.await else {
                return;
            };
            let answer = answer.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

/// 主人发的一条消息；`thread_root` 是它所属话题的根。
///
/// 时间戳逐条递增（进程级计数，起点随意）：和真实时间线一样从旧到新；要钉住先后
/// （停机积压）的测试用 [`owner_message_at`] 自己给。
pub fn owner_message(event_id: &str, text: &str, thread_root: Option<&str>) -> Message {
    static NEXT: AtomicU64 = AtomicU64::new(1000);
    owner_message_at(
        event_id,
        text,
        thread_root,
        NEXT.fetch_add(1, Ordering::Relaxed),
    )
}

/// 同 [`owner_message`]，时间戳自定：停机积压的测试要拿它跟已读位置比先后。
pub fn owner_message_at(
    event_id: &str,
    text: &str,
    thread_root: Option<&str>,
    timestamp_ms: u64,
) -> Message {
    Message {
        event_id: Some(event_id.parse().unwrap()),
        sender: "@owner:example.org".parse::<OwnedUserId>().unwrap(),
        sender_name: String::new(),
        is_own: false,
        timestamp_ms,
        status: SendStatus::Sent,
        body: MessageBody::Text(text.into()),
        custom: Default::default(),
        mentions: Vec::new(),
        agent_run: None,
        thread_root: thread_root.map(|root| root.parse().unwrap()),
        edited: false,
        read_by: Vec::new(),
        reply: None,
        group: None,
        can_redact: false,
    }
}

/// 主人发的一条带内容的消息（图片、文件等），其余字段同 [`owner_message`]。
pub fn owner_message_with(event_id: &str, body: MessageBody) -> Message {
    let mut message = owner_message(event_id, "", None);
    message.body = body;
    message
}

/// 等到异步断言成立，最久 5 秒。
pub async fn wait_for<F>(mut check: impl FnMut() -> F)
where
    F: Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待副作用超时");
}

/// 等到断言成立，最久 5 秒。
pub async fn wait_until(mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待副作用超时");
}
