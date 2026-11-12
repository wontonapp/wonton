//! 假核心收到的请求和请求行：等超时时用来说明卡在哪个请求上。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 是不是房间列表那条 Sliding Sync 的请求。连接号在 `room-list-a` / `room-list-b` 之间轮换，早先是固定的
/// `room-list`。
pub fn is_room_list(conn_id: &serde_json::Value) -> bool {
    matches!(
        conn_id.as_str(),
        Some("room-list-a" | "room-list-b" | "room-list")
    )
}

/// 一条经过假核心的 Matrix 请求：什么时候收到的、请求行、答完没有。等超时时用来说明卡在哪个请求上。
struct Request {
    id: usize,
    at: std::time::Instant,
    line: String,
    done: AtomicBool,
}

/// 最近收到的请求，最多留这么多条。
const RECENT_REQUESTS: usize = 64;

static REQUESTS: Mutex<VecDeque<Request>> = Mutex::new(VecDeque::new());
static NEXT_REQUEST_ID: AtomicUsize = AtomicUsize::new(0);

/// 记下收到的请求，返回它的编号，答完时交给 [`request_done`]。
pub fn request_arrived(head: &str) -> usize {
    let id = NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst);
    let line = head.lines().next().unwrap_or_default().to_owned();
    let mut requests = REQUESTS.lock().unwrap();
    requests.push_back(Request {
        id,
        at: std::time::Instant::now(),
        line,
        done: AtomicBool::new(false),
    });
    while requests.len() > RECENT_REQUESTS {
        requests.pop_front();
    }
    id
}

/// 请求答完了（连接关掉也算答完）。
pub fn request_done(id: usize) {
    if let Some(request) = REQUESTS
        .lock()
        .unwrap()
        .iter()
        .find(|request| request.id == id)
    {
        request.done.store(true, Ordering::SeqCst);
    }
}

/// `since` 之后收到、还没答完的请求和各自等了多久，等超时时写进失败说明。同步起步卡住时，这里就是卡着的那一条。
/// 按 `since` 掐掉等之前就答别的测试的请求：一个进程里同时跑着好几条用例。
pub fn pending_requests(since: std::time::Instant) -> String {
    let requests = REQUESTS.lock().unwrap();
    let mut pending: Vec<String> = requests
        .iter()
        .filter(|request| !request.done.load(Ordering::SeqCst) && request.at >= since)
        .map(|request| format!("{:?} {}", request.at.elapsed(), request.line))
        .collect();
    if pending.is_empty() {
        let last: Vec<String> = requests
            .iter()
            .rev()
            .take(5)
            .map(|request| format!("{:?} {}", request.at.elapsed(), request.line))
            .collect();
        return format!("没有，最后收到的：{}", last.join("；"));
    }
    pending.reverse();
    pending.join("；")
}
