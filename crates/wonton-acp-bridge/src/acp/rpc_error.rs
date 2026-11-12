//! ACP JSON-RPC 的错误对象：从 Agent 的错误响应里取出代码和给用户看的文字。

use serde_json::Value;

use crate::error::Detail;

/// 错误文字的上限：整段直接进聊天，不能刷屏。
const MAX_TEXT_BYTES: usize = 2 * 1024;

#[derive(Debug)]
pub(super) struct RpcError {
    pub(super) code: i64,
    pub(super) message: String,
    /// 桥自己写的整句：Agent 没带 message 时用它，聊天里不会露出中文常量。
    pub(super) detail: Option<Detail>,
}

impl RpcError {
    /// Agent 报错的真正原因常在 `error.data` 里（实测 omp：message 只有笼统的
    /// "Internal error"，`data.details` 才是要给用户的说明）。data 是字符串、或带字符串
    /// 字段 `details`/`message` 的对象时，把它接在 message 后面（换行分隔）；其余形状照旧。
    pub(super) fn from_wire(error: &Value) -> Self {
        let missing = error["message"]
            .as_str()
            .filter(|text| !text.is_empty())
            .is_none();
        let message = error["message"]
            .as_str()
            .unwrap_or("the ACP request failed");
        let detail = match error.get("data") {
            Some(Value::String(detail)) => Some(detail.as_str()),
            Some(data) if data.is_object() => data["details"]
                .as_str()
                .or_else(|| data["message"].as_str()),
            _ => None,
        };
        let mut text = message.to_owned();
        if let Some(detail) = detail.filter(|detail| !detail.is_empty() && *detail != message) {
            text.push('\n');
            text.push_str(detail);
        }
        if text.len() > MAX_TEXT_BYTES {
            let mut end = MAX_TEXT_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        Self {
            code: error["code"].as_i64().unwrap_or(-32603),
            message: text,
            detail: missing.then(|| Detail::msg("acp-request-failed")),
        }
    }
}
