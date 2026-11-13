//! 桥的行为测试用的替身：内存房间记录所有 Matrix 侧副作用，假核心收确认卡片、
//! 调用链报告和出箱（`/agent/permissions`、`/agent/chains`、`/agent/outbox`），
//! ACP 侧用 `acp::test_support::FakeAgent` 的 Manual 场景驱动。

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::{Stream, StreamExt, stream};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use wonton_matrix::{Message, MessageBody, OwnedUserId, SendStatus};
use wonton_protocol::OutboxItem;

use crate::error::{Detail, Error, Result};
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
    /// 重发了被拒收的消息。
    ResendRefused,
    /// 对这条消息发的已读回执。
    Receipt(String),
    Redact(String),
    State(String),
    /// 一条 Agent 进度状态消息（ADR-0082），内容原样。
    RunStatus {
        event_id: String,
        thread: Option<String>,
        content: Value,
    },
    /// 一条旁聊结果行（ADR-0093），内容原样。
    SideChatResult {
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
    /// 状态写入要回的失败：设了之后 set_state 一律返回这个错误（模拟 Bot 对房间没有写
    /// 状态的 power，403）。
    state_failure: Arc<Mutex<Option<String>>>,
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
            state_failure: Arc::new(Mutex::new(None)),
        }
    }

    /// 登记一个可下载的媒体句柄。
    pub fn serve_media(&self, token: &str, bytes: Vec<u8>) {
        self.media.lock().unwrap().insert(token.to_owned(), bytes);
    }

    /// 让状态写入一律失败（模拟 Bot 没有写状态的 power，403）。
    pub fn forbid_state_writes(&self, message: &str) {
        *self.state_failure.lock().unwrap() = Some(message.to_owned());
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
                | Effect::ResendRefused
                | Effect::Receipt(_)
                | Effect::Redact(_)
                | Effect::State(_)
                | Effect::RunStatus { .. }
                | Effect::SideChatResult { .. }
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

    /// 发过已读回执的消息（按发送顺序）。
    pub fn receipts(&self) -> Vec<String> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Receipt(event_id) => Some(event_id),
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

    async fn send_side_chat_result(&self, content: Value) -> Result<()> {
        self.record(Effect::SideChatResult { content });
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

    async fn resend_refused(&self) -> Result<usize> {
        self.record(Effect::ResendRefused);
        Ok(1)
    }

    async fn read_receipt(&self, event_id: &str) -> Result<()> {
        self.record(Effect::Receipt(event_id.to_owned()));
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
        let failure = self.state_failure.lock().unwrap().clone();
        if let Some(message) = failure {
            // 和 From<MatrixError> 一致：403 落到 Matrix(Data)，message 是服务器给的说明。
            return Err(Error::Matrix(Detail::data(format!(
                "M_FORBIDDEN: {message}"
            ))));
        }
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

/// 假的核心：确认卡片（`/agent/permissions`）、调用链（`/agent/chains`）和出箱
/// （`/agent/outbox`）都在 `WONTON_MCP_URL` 这一个地址后面，和真核心同一道门。
pub struct FakeCore {
    url: String,
    /// 确认卡片的请求体（按到达顺序）。
    requests: Arc<Mutex<Vec<Value>>>,
    /// 调用链收到的报告体（Start/End，按到达顺序）。
    chain_reports: Arc<Mutex<Vec<Value>>>,
    /// 出箱收到的回报体（按到达顺序）。
    receipts: Arc<Mutex<Vec<Value>>>,
    /// 等回答的确认卡片；测试按先来后到回答。
    answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>>,
    /// 下一次调用链报告要回的（状态码，错误体）；空了就回默认的放行。
    chain_replies: Arc<Mutex<VecDeque<(u16, Value)>>>,
    /// 等取走的出箱项；`feed` 推进来，长轮询取走。
    outbox: Arc<Mutex<VecDeque<Value>>>,
    /// 出箱的版本号：推进项就加一，长轮询挂在它上面（和核心的 `Outbox` 同一个做法）。
    outbox_revision: Arc<watch::Sender<u64>>,
    /// 等回答时对端断开的确认长轮询数（停链取消要断开它们）。
    dropped_permissions: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl FakeCore {
    pub async fn start() -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let chain_reports = Arc::new(Mutex::new(Vec::new()));
        let receipts = Arc::new(Mutex::new(Vec::new()));
        let answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(VecDeque::new()));
        let chain_replies = Arc::new(Mutex::new(VecDeque::new()));
        let outbox = Arc::new(Mutex::new(VecDeque::new()));
        let outbox_revision = Arc::new(watch::channel(0).0);
        let dropped_permissions = Arc::new(AtomicUsize::new(0));
        let chain_counter = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn(serve_core(
            listener,
            requests.clone(),
            chain_reports.clone(),
            receipts.clone(),
            answers.clone(),
            dropped_permissions.clone(),
            chain_replies.clone(),
            outbox.clone(),
            outbox_revision.clone(),
            chain_counter.clone(),
        ));
        Self {
            // 带总线令牌的地址：桥从它推导确认、调用链和出箱的端点。
            url: format!("http://{address}/mcp/test-bus"),
            requests,
            chain_reports,
            receipts,
            answers,
            chain_replies,
            outbox,
            outbox_revision,
            dropped_permissions,
            task,
        }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }

    /// 调用链收到的报告体（Start/End，按到达顺序）。
    pub fn chain_reports(&self) -> Vec<Value> {
        self.chain_reports.lock().unwrap().clone()
    }

    pub fn receipts(&self) -> Vec<Value> {
        self.receipts.lock().unwrap().clone()
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

    /// 等回答时被对端断开的确认长轮询数。
    pub fn dropped_permissions(&self) -> usize {
        self.dropped_permissions.load(Ordering::SeqCst)
    }

    /// 下一次调用链报告回这个（状态码，体）；体是 [`ChainError`] 的 JSON 或任意错误。
    pub fn push_chain_reply(&self, status: u16, body: Value) {
        self.chain_replies.lock().unwrap().push_back((status, body));
    }

    /// 给出箱排一项，长轮询会取走它。
    pub fn feed(&self, item: OutboxItem) {
        self.feed_raw(serde_json::to_value(item).unwrap());
    }

    /// 给出箱排一项原始 JSON（不认识的项的测试用）。
    pub fn feed_raw(&self, item: Value) {
        self.outbox.lock().unwrap().push_back(item);
        self.outbox_revision.send_modify(|revision| *revision += 1);
    }

    pub fn finish(&self) {
        self.task.abort();
    }
}

/// 一次 HTTP 请求：方法、路径、体。测试替身只看这些。
struct RawRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
async fn serve_core(
    listener: TcpListener,
    card_requests: Arc<Mutex<Vec<Value>>>,
    chain_reports: Arc<Mutex<Vec<Value>>>,
    receipts: Arc<Mutex<Vec<Value>>>,
    answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>>,
    dropped: Arc<AtomicUsize>,
    chain_replies: Arc<Mutex<VecDeque<(u16, Value)>>>,
    outbox: Arc<Mutex<VecDeque<Value>>>,
    outbox_revision: Arc<watch::Sender<u64>>,
    chain_counter: Arc<AtomicU64>,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(handle_connection(
            stream,
            card_requests.clone(),
            chain_reports.clone(),
            receipts.clone(),
            answers.clone(),
            dropped.clone(),
            chain_replies.clone(),
            outbox.clone(),
            outbox_revision.clone(),
            chain_counter.clone(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    stream: tokio::net::TcpStream,
    card_requests: Arc<Mutex<Vec<Value>>>,
    chain_reports: Arc<Mutex<Vec<Value>>>,
    receipts: Arc<Mutex<Vec<Value>>>,
    answers: Arc<Mutex<VecDeque<oneshot::Sender<Value>>>>,
    dropped: Arc<AtomicUsize>,
    chain_replies: Arc<Mutex<VecDeque<(u16, Value)>>>,
    outbox: Arc<Mutex<VecDeque<Value>>>,
    outbox_revision: Arc<watch::Sender<u64>>,
    chain_counter: Arc<AtomicU64>,
) {
    let Some((mut stream, request)) = read_request(stream).await else {
        return;
    };
    if request.path.starts_with("/agent/chains") {
        serve_chains(
            &request,
            &chain_reports,
            &chain_replies,
            &chain_counter,
            &mut stream,
        )
        .await;
    } else if request.path.starts_with("/agent/outbox") && request.method == "GET" {
        serve_outbox(&outbox, &outbox_revision, &mut stream).await;
    } else if request.path.starts_with("/agent/outbox") {
        if let Ok(receipt) = serde_json::from_slice::<Value>(&request.body) {
            receipts.lock().unwrap().push(receipt);
        }
        respond(&mut stream, 200, "{}").await;
    } else {
        serve_permission(&request, &card_requests, &answers, &dropped, stream).await;
    }
}

async fn serve_chains(
    request: &RawRequest,
    reports: &Mutex<Vec<Value>>,
    replies: &Mutex<VecDeque<(u16, Value)>>,
    counter: &AtomicU64,
    stream: &mut tokio::net::TcpStream,
) {
    if let Ok(report) = serde_json::from_slice::<Value>(&request.body) {
        reports.lock().unwrap().push(report);
    }
    let reply = replies.lock().unwrap().pop_front();
    match reply {
        Some((status, body)) => {
            respond(stream, status, &body.to_string()).await;
        }
        None => {
            let chain = format!("chain-{}", counter.fetch_add(1, Ordering::SeqCst) + 1);
            let grant = serde_json::json!({"chain": chain, "layer": 1});
            respond(stream, 200, &grant.to_string()).await;
        }
    }
}

async fn serve_outbox(
    outbox: &Mutex<VecDeque<Value>>,
    revision: &watch::Sender<u64>,
    stream: &mut tokio::net::TcpStream,
) {
    // 先订阅再看队列：中间推进来的项不会漏（和核心的长轮询同一个顺序）。
    let mut watcher = revision.subscribe();
    let body = loop {
        if let Some(item) = outbox.lock().unwrap().pop_front() {
            break item.to_string();
        }
        // 长轮询：推进来就给，挂到超时给 null（和核心的 AGENT_OUTBOX_WAIT_SECS 一个意思）。
        match tokio::time::timeout(
            Duration::from_secs(wonton_protocol::AGENT_OUTBOX_WAIT_SECS),
            watcher.changed(),
        )
        .await
        {
            Ok(Ok(())) => {}
            _ => break "null".to_owned(),
        }
    };
    respond(stream, 200, &body).await;
}

async fn serve_permission(
    request: &RawRequest,
    records: &Mutex<Vec<Value>>,
    answers: &Mutex<VecDeque<oneshot::Sender<Value>>>,
    dropped: &AtomicUsize,
    stream: tokio::net::TcpStream,
) {
    if let Ok(request) = serde_json::from_slice::<Value>(&request.body) {
        records.lock().unwrap().push(request);
    }
    let (sender, receiver) = oneshot::channel();
    answers.lock().unwrap().push_back(sender);
    // 读、写各拿一半：等断开和写响应要交替进行，同一份流借不来。
    let (mut read_half, mut write_half) = tokio::io::split(stream);
    let mut eof_buf = [0u8; 1];
    tokio::select! {
        answer = receiver => {
            if let Ok(answer) = answer {
                respond(&mut write_half, 200, &answer.to_string()).await;
            }
        }
        eof = read_half.read(&mut eof_buf) => {
            // 客户端在等回答时断开（桥取消了未决的权限请求）：记下来，不再等。
            if matches!(eof, Ok(0) | Err(_)) {
                dropped.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
}

/// 读一条请求：一行方法行、头（只看 Content-Length）、体。返回流原样，调用方还要写响应。
async fn read_request(
    stream: tokio::net::TcpStream,
) -> Option<(tokio::net::TcpStream, RawRequest)> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await.ok()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return None;
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
        return None;
    }
    let stream = reader.into_inner();
    Some((stream, RawRequest { method, path, body }))
}

async fn respond<W: tokio::io::AsyncWrite + Unpin>(stream: &mut W, status: u16, body: &str) {
    let reason = match status {
        403 => "Forbidden",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
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
        thread_replies: None,
        edited: false,
        read_by: Vec::new(),
        reply: None,
        group: None,
        can_redact: false,
    }
}

/// Bot 自己发的一条消息，`status` 是它现在的发送状态。
pub fn own_message(text: &str, status: SendStatus) -> Message {
    Message {
        event_id: None,
        sender: "@bot:example.org".parse::<OwnedUserId>().unwrap(),
        is_own: true,
        status,
        ..owner_message("$unused", text, None)
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
