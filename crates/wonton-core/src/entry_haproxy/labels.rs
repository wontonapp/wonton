//! 插件在入口上的标签：取插件 ID 的最后一段，装的时候定下，写进安装记录，升级和重启都不变。
//!
//! 标签会进证书名和 HAProxy 配置，字符集和长度卡死，重名和保留名往后加序号。

use crate::plugin::manifest::PluginId;

/// 入口自检专用的标签，插件的标签不许和它撞。
pub const PROBE_LABEL: &str = "probe";
/// 保留名：`lan` 留给局域网名字的形式（`<标签>.lan.<后缀>`）。
pub const RESERVED: [&str; 2] = ["lan", PROBE_LABEL];

const MAX_LABEL: usize = 63;
const MAX_ATTEMPTS: u32 = 10_000;

/// 标签的字符面和长度：`[a-z0-9-]`，1 到 63 字节，首尾是字母或数字。
pub fn validate_name(label: &str) -> Result<(), String> {
    let valid = (1..=MAX_LABEL).contains(&label.len())
        && label.starts_with(|c: char| c.is_ascii_alphanumeric())
        && label.ends_with(|c: char| c.is_ascii_alphanumeric())
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!("标签 {label:?} 不合法"))
    }
}

/// 插件标签：字符面之外还不能是保留名（`lan`、探测标签本身）。
pub fn validate_label(label: &str) -> Result<(), String> {
    validate_name(label)?;
    if RESERVED.contains(&label) {
        return Err(format!("标签 {label:?} 是保留名"));
    }
    Ok(())
}

/// 已经有的标签留着（升级、重启），没有就分配一个：重名、保留名依次加 `-2`、`-3`。
pub fn ensure(existing: Option<&str>, id: &PluginId, taken: &[String]) -> Result<String, String> {
    if let Some(existing) = existing {
        validate_label(existing)?;
        return Ok(existing.to_owned());
    }
    assign(&id.to_string(), taken)
}

/// 给插件 ID 分配新标签。
pub fn assign(id: &str, taken: &[String]) -> Result<String, String> {
    let base = base_of(id)?;
    for attempt in 1..=MAX_ATTEMPTS {
        let candidate = numbered(&base, attempt);
        if !RESERVED.contains(&candidate.as_str()) && !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!("给插件 {id:?} 找不到空闲标签"))
}

/// 插件 ID 的最后一段。
fn base_of(id: &str) -> Result<String, String> {
    let segment = id.rsplit('.').next().unwrap_or(id);
    let cleaned: String = segment
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        return Err(format!("插件 {id:?} 的最后一段没有可用字符"));
    }
    Ok(trimmed.to_owned())
}

/// 第 `attempt` 个候选：首个是原名，之后加 `-2`、`-3`；超过 63 字节时截短前半段。
fn numbered(base: &str, attempt: u32) -> String {
    if attempt == 1 {
        return truncate(base, MAX_LABEL);
    }
    let suffix = format!("-{attempt}");
    let room = MAX_LABEL.saturating_sub(suffix.len());
    format!("{}{suffix}", truncate(base, room))
}

fn truncate(text: &str, max: usize) -> String {
    let cut: String = text.chars().take(max).collect();
    cut.trim_end_matches('-').to_owned()
}

impl crate::plugin::PluginManager {
    /// 安装时写进记录的标签：有了照用，没有就新分一个。
    /// 记录读写失败保留「存储」类别，别混成输入非法。
    pub(crate) fn install_entry_label(
        &self,
        id: &PluginId,
    ) -> Result<Option<String>, crate::plugin::InstallError> {
        let taken: Vec<String> = self
            .store()
            .load_all()?
            .into_iter()
            .filter_map(|file| file.record.ok()?.entry_label)
            .collect();
        let existing = self.store().load(id)?.and_then(|record| record.entry_label);
        ensure(existing.as_deref(), id, &taken)
            .map(Some)
            .map_err(crate::plugin::InstallError::Invalid)
    }
}
