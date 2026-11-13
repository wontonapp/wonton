//! 每个 Bot 的网页身份和按请求者计算的访问结果（ADR-0045）。
//!
//! 清单 `webui.service` 指向 `bots.services` 时使用此契约。网关依据当前会话和插件已有的网页授权开关判定访问，
//! 不按 Bot 归属另设权限。网页会话绑定完整主机名，不设父域 Cookie。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::BotRef;

/// 非默认 Bot 的网页标签前缀；合法插件 ID 派生的 slug 不含连续连字符，不会占用此命名空间。
pub const BOT_WEB_LABEL_PREFIX: &str = "bot--";

/// 已校验的 Bot 身份对应的规范网页标签，不含基础域名；默认 Bot 沿用插件 slug。
///
/// 非默认 Bot：`bot--<bot>-<slug 前缀>-<摘要>`。摘要是 SHA-256(`plugin_id` UTF-8 + 一个 NUL 字节 +
/// `bot` UTF-8) 的前 8 位小写十六进制；只把 slug 截到 `48 - bot.len()` 字节，整个标签不超过 63 字节。
/// Bot 名不截短，不静默改名或回落到默认 Bot；登记必须用 [`crate::bot_web_registration`] 检查短摘要撞名。
pub fn bot_web_label(bot: &BotRef) -> String {
    if bot.bot == "default" {
        return bot.plugin_id.replace('.', "-");
    }
    let mut hash = Sha256::new();
    hash.update(bot.plugin_id.as_bytes());
    hash.update([0]);
    hash.update(bot.bot.as_bytes());
    let digest = hash.finalize();
    let slug_bytes = (48 - bot.bot.len()).min(bot.plugin_id.len());
    let mut label = String::with_capacity(15 + bot.bot.len() + slug_bytes);
    label.push_str(BOT_WEB_LABEL_PREFIX);
    label.push_str(&bot.bot);
    label.push('-');
    label.extend(
        bot.plugin_id
            .bytes()
            .take(slug_bytes)
            .map(|byte| if byte == b'.' { '-' } else { char::from(byte) }),
    );
    label.push('-');
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in &digest[..4] {
        label.push(char::from(HEX[(byte >> 4) as usize]));
        label.push(char::from(HEX[(byte & 15) as usize]));
    }
    label
}

/// `GET`：按当前请求者返回 [`BotWebView`]；参数来自核心记录的 [`BotRef`]，不从 Matrix 用户名反推。
///
/// 未登录回 401 `unauthenticated`；插件未开放给家人回 403 `not_shared`；插件网络来源回 403 `permission_denied`；没有这个 Bot 回
/// 404 `not_found`；没有独立 Bot 网页回 404 `no_webui`；容器没就绪回 503 `not_ready`。
/// 成功返回 `Ready`，错误响应使用 `Unavailable` 的原因类别，不借用默认 Bot 的地址。
pub fn bot_web_path(bot: &BotRef) -> String {
    format!("/_wonton/api/bots/{}/{}/webui", bot.plugin_id, bot.bot)
}

/// Bot 资料页的网页状态；文字按类别写在原生壳的四语言资源中，不把地址当作授权凭据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BotWebView {
    Ready {
        page: BotWebPage,
    },
    Unavailable {
        reason: BotWebFailure,
    },
    /// 比客户端新的状态不能授予网页访问。
    #[serde(other)]
    Other,
}

/// 独立 Bot 网页的元数据；导航使用 [`crate::TabRef::Bot`]，URL 只从同一身份已核实的 `BotView.web` 取得。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotWebPage {
    pub bot: BotRef,
    /// 网关给出的规范 origin 加清单 `webui.path`；壳原样打开，不自行拼域名或替换 Bot。
    pub url: String,
    pub cross_origin_isolated: bool,
}

/// 网关和 Bot 资料页共用的不能打开原因；未知类别一律不能打开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BotWebFailure {
    Unauthenticated,
    NotShared,
    PermissionDenied,
    NotFound,
    NoWebui,
    NotReady,
    #[serde(other)]
    Other,
}

#[cfg(test)]
#[path = "bot_web_tests.rs"]
mod tests;
