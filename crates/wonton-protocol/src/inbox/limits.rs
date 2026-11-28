/// 一页 HTML 最大多少字节。
pub const MAX_INBOX_PAGE_BYTES: usize = 1024 * 1024;

/// 一张卡最多几题、每题最多几个选项。
pub const MAX_INBOX_QUESTIONS: usize = 4;
pub const MAX_INBOX_OPTIONS: usize = 4;
pub const MIN_INBOX_OPTIONS: usize = 2;

/// 一条条目上最多几个动作。
pub const MAX_INBOX_ACTIONS: usize = 3;

/// 网络名单最多几个域名。
pub const MAX_INBOX_NETWORK_DOMAINS: usize = 20;

/// 同时挂着的 choice/confirm/recap 合计上限、page 上限。
pub const MAX_PENDING_INBOX_ITEMS: usize = 5;
pub const MAX_PENDING_INBOX_PAGES: usize = 3;

/// 新建频率上限（按发送方算）。
pub const MAX_INBOX_PER_MINUTE: usize = 10;
pub const MAX_INBOX_PER_DAY: usize = 200;

/// 字段上限（字符数）。超了报 `invalid_input`，不截断。
pub const MAX_INBOX_TITLE_CHARS: usize = 120;
pub const MAX_INBOX_BODY_CHARS: usize = 2000;
pub const MAX_INBOX_DETAIL_CHARS: usize = 8000;
pub const MAX_INBOX_SUMMARY_CHARS: usize = 400;
pub const MAX_INBOX_CAPTION_CHARS: usize = 120;
pub const MAX_INBOX_HEADER_CHARS: usize = 12;
pub const MAX_INBOX_QUESTION_CHARS: usize = 200;
pub const MAX_INBOX_OPTION_LABEL_CHARS: usize = 40;
pub const MAX_INBOX_OPTION_DESCRIPTION_CHARS: usize = 100;
pub const MAX_INBOX_OTHER_CHARS: usize = 2000;
pub const MAX_INBOX_NOTE_CHARS: usize = 1000;
pub const MAX_INBOX_COMMENT_CHARS: usize = 200;
pub const MAX_INBOX_ACTION_LABEL_CHARS: usize = 40;
pub const MAX_INBOX_DISPLAY_NAME_CHARS: usize = 80;
pub const MAX_INBOX_SENDER_ID_CHARS: usize = 128;
pub const MAX_INBOX_DEDUPE_KEY_CHARS: usize = 128;
pub const MAX_INBOX_PROGRESS_TOTAL: u32 = u32::MAX;

/// 条目 id 的前缀：q = 问题卡，c = 确认，p = 页面，r = 收尾，n = 通知。
pub const INBOX_ID_PREFIXES: &[char] = &['q', 'c', 'p', 'r', 'n'];

/// 页面资源必须是 `ui://` 的 MCP Apps 资源。
pub const INBOX_RESOURCE_SCHEME: &str = "ui://";
