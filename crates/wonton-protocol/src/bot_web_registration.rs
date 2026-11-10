//! Bot 网页登记的地址冲突契约；登记表由服务器持有。

use crate::{BotRef, bot_web_label};

/// 同一标签被另一个 Bot 占用；登记失败，不覆盖现有地址、不自动换名。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Bot webpage label {label} is already registered to {registered:?}, not {requested:?}")]
pub struct BotWebConflict {
    pub label: String,
    pub registered: BotRef,
    pub requested: BotRef,
}

/// 登记前查出的同标签占用者。`None` 表示空闲；同一身份重复登记保持幂等。
///
/// 调用方须把查询和登记放在同一把锁内；冲突时保留原登记并向用户返回 409 `conflict`。
/// 标签仅负责寻址，不能替代身份或网页授权检查。
pub fn bot_web_registration(
    requested: &BotRef,
    registered: Option<&BotRef>,
) -> Result<String, BotWebConflict> {
    let label = bot_web_label(requested);
    if let Some(registered) = registered.filter(|registered| *registered != requested) {
        return Err(BotWebConflict {
            label,
            registered: registered.clone(),
            requested: requested.clone(),
        });
    }
    Ok(label)
}
