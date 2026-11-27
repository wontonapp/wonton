//! 连接、读资源、列工具的期限和大小上限。

use std::time::Duration;

pub(super) use super::super::bounded_http::MAX_TOOLS_BYTES;

pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 插件接受了连接却不回握手：不能一直等。
pub(super) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 读一个资源最多这么久。
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// 列一个插件的工具（包括没连上时的握手）最多这么久。
pub(super) const TOOLS_TIMEOUT: Duration = Duration::from_secs(15);

/// 见模块说明。
pub(super) const MAX_TIMEOUTS: u32 = 3;

/// 插件的一个响应最多这么多字节：`resources/read` 的 32 MiB 经 base64 变成 43 MiB，再加 JSON。
pub(super) const MAX_MESSAGE_BYTES: usize = 48 << 20;

/// 一个插件的工具列表最多翻这么多页、这么多个工具，总共 [`MAX_TOOLS_BYTES`]：rmcp 的 `list_all_tools` 一直跟着 `next_cursor` 翻，
/// 而每个 Agent 都会并发地列所有插件，一个插件就能让核心用完内存。
pub(super) const MAX_TOOL_PAGES: usize = 50;

pub(super) const MAX_TOOLS: usize = 2_000;

/// 读一个资源最长要多久：连接（包括握手）加读取。调用方的期限要比它长，这里的超时才记得上（[`super::Upstreams::timed_out`]）。
pub const READ_BUDGET: Duration = CONNECT_TIMEOUT
    .saturating_add(HANDSHAKE_TIMEOUT)
    .saturating_add(READ_TIMEOUT);
