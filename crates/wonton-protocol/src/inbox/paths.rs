/// `GET <核心地址>/_wonton/api/inbox`：列表，带 `after=<revision>` 时长轮询，最多 [`INBOX_WAIT_SECS`] 秒。
pub const INBOX_PATH: &str = "/_wonton/api/inbox";

/// 列表长轮询最多挂住多久。客户端的超时要比它长。
pub const INBOX_WAIT_SECS: u64 = 25;

/// 页面短令牌的有效期（秒）。重载页面时 App 重新向详情接口要一个。
pub const INBOX_PAGE_TOKEN_TTL_SECS: u64 = 300;

/// 异步条目答完后多久内可以撤回重答（秒）。
pub const INBOX_REANSWER_WINDOW_SECS: u64 = 60;

/// MCP 总线上核心自己的收件箱工具（ADR-0119 第 3 节）。
pub const INBOX_ASK_TOOL: &str = "wonton__inbox_ask";
pub const INBOX_PRESENT_TOOL: &str = "wonton__inbox_present";
pub const INBOX_RECAP_TOOL: &str = "wonton__inbox_recap";
pub const INBOX_CONFIRM_TOOL: &str = "wonton__inbox_confirm";
pub const INBOX_NOTICE_TOOL: &str = "wonton__inbox_notify";
pub const INBOX_WAIT_TOOL: &str = "wonton__inbox_wait";
pub const INBOX_CANCEL_TOOL: &str = "wonton__inbox_cancel";
pub const INBOX_UPDATE_TOOL: &str = "wonton__inbox_update";

/// 一条条目的路径。
pub fn inbox_item_path(id: &str) -> String {
    format!("{INBOX_PATH}/{id}")
}

/// 作答接口的路径。
pub fn inbox_answer_path(id: &str) -> String {
    format!("{INBOX_PATH}/{id}/answer")
}

/// 撤回答案、在 60 秒内重答的路径。
pub fn inbox_withdraw_path(id: &str) -> String {
    format!("{INBOX_PATH}/{id}/answer/withdraw")
}

/// 已读、稍后、取消的路径。
pub fn inbox_read_path(id: &str) -> String {
    format!("{INBOX_PATH}/{id}/read")
}

pub fn inbox_snooze_path(id: &str) -> String {
    format!("{INBOX_PATH}/{id}/snooze")
}

pub fn inbox_cancel_path(id: &str) -> String {
    format!("{INBOX_PATH}/{id}/cancel")
}

/// 页面接口的路径；`token` 是详情接口发的短令牌。
pub fn inbox_page_path(id: &str, token: &str) -> String {
    format!("{INBOX_PATH}/{id}/page?token={token}")
}

/// worker 事件入口、取答案、回报投递结果。
pub const INBOX_WORKERS_PATH: &str = "/_wonton/api/inbox/workers";
pub fn inbox_worker_path(worker_id: &str) -> String {
    format!("{INBOX_WORKERS_PATH}/{worker_id}")
}
pub fn inbox_worker_resolve_path(worker_id: &str) -> String {
    format!("{INBOX_WORKERS_PATH}/{worker_id}/resolve")
}
