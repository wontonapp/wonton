//! 待钉窗口（ADR-0112 §5b）：安装时给 `allow_invalid_tls_certificates` 定初值，运行期在钉齐或窗口到期后关掉。
//!
//! 窗口 = 两个核心版本：`since_version` 是配对成立（或老记录升级补写）时的核心版本，次版本涨到起点 +2
//! 或主版本更大就到期。只有“已配对、还没钉住”的记录需要窗口；钉齐了不需要，到期了也不开（保持待钉、
//! 如实报“对端还没升级”）。
//!
//! 开发构建（`dev`、`dev+<sha>`）没有可比的版本：一律按没到期算，窗口保持打开。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context as _;
use serde::Deserialize;

/// 现在的窗口状态。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WindowState {
    /// 还有没到期、没钉住的记录：`allow_invalid_tls_certificates` 要开着。
    pub needed: bool,
    /// 窗口已到期、还没钉住的记录数：如实报“对端还没升级”。
    pub expired: usize,
}

/// 读 `core/peers.json` 判现在的窗口状态。没有数据目录、还没有记录文件都按不需要窗口。
pub(crate) fn window_state(
    data_dir: Option<&Path>,
    current_version: &str,
) -> anyhow::Result<WindowState> {
    let Some(data_dir) = data_dir else {
        return Ok(WindowState::default());
    };
    let path = data_dir.join("core/peers.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(WindowState::default()),
        Err(e) => return Err(e).with_context(|| format!("读取 {}", path.display())),
    };
    let stored: Stored =
        serde_json::from_str(&text).with_context(|| format!("解析 {}", path.display()))?;
    let mut state = WindowState::default();
    for record in stored.peers.values() {
        if record.phase != Some(crate::peers::Phase::Paired) {
            continue;
        }
        // 钉住的状态且有 pin 才算真钉住；配了 pinned 却没 pin、或没有 pin_state 的老记录都还要窗口。
        if record.pin_state == Some(PinState::Pinned) && !record.peer_pin.is_empty() {
            continue;
        }
        let since_version = match &record.pin_state {
            Some(PinState::Pending { since_version }) => since_version.as_str(),
            _ => "",
        };
        if window_expired(since_version, current_version) {
            state.expired += 1;
        } else {
            state.needed = true;
        }
    }
    Ok(state)
}

#[derive(Deserialize, Default)]
struct Stored {
    #[serde(default)]
    peers: BTreeMap<String, Record>,
}

#[derive(Deserialize, Default)]
struct Record {
    phase: Option<crate::peers::Phase>,
    /// 核心写的是 `[{der, spki_sha256, pinned_at_ms}]`（`PeerPin`）；这里只关心有没有。
    #[serde(default)]
    peer_pin: Vec<serde_json::Value>,
    /// `pinned` 或 `{"pending":{"since_version":…}}`；老记录没有这个字段。
    pin_state: Option<PinState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PinState {
    Pinned,
    Pending {
        #[serde(default)]
        since_version: String,
    },
}

/// 待钉窗口是否已到期（ADR-0112 §5b：窗口 = 两个核心版本）。`since_version` 是窗口起点，`current_version`
/// 是现在的核心版本：次版本涨到起点 +2（或主版本更大）就到期；主版本相同、次版本没涨够时没到期，降级
/// （当前比起点旧）也没到期。
///
/// 开发构建（`dev`、`dev+<sha>`）没有可比的版本：任意一端不是 `主.次[.补]` 一律按**没到期**算——窗口
/// 保持打开，放宽着也不断线；日志里如实报不知道先后。
///
/// 和 `peers::pin::window_expired` 同一条判据；合并后换成那一个（见 PR #1123 的提交）。
fn window_expired(since_version: &str, current_version: &str) -> bool {
    let (Some((since_major, since_minor)), Some((current_major, current_minor))) =
        (major_minor(since_version), major_minor(current_version))
    else {
        return false;
    };
    current_major > since_major
        || (current_major == since_major && current_minor >= since_minor + 2)
}

/// `主.次` 两个数字；补丁号可省。开发构建和非数字版本回 `None`。
fn major_minor(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Tuwunel 配置里那一行。
const ALLOW_INVALID: &str = "allow_invalid_tls_certificates";

/// 上一次自动写进 `allow_invalid_tls_certificates` 的值。跨重启用它区分“主人手改”和“上次自动写的”。
pub(crate) fn last_applied_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("core/window-applied")
}

/// 主人手改顶掉自动的值。存下来，重启后仍认得出，不把主人的关闭改回去。
pub(crate) fn manual_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("core/window-manual")
}

/// 读主人顶掉的值；没有或读不懂都当没有。
pub(crate) fn load_manual(data_dir: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(manual_path(data_dir)).ok()?;
    match text.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// 记下主人顶掉的值。
pub(crate) async fn store_manual(data_dir: &Path, value: bool) {
    let path = manual_path(data_dir);
    if let Some(dir) = path.parent()
        && let Err(error) = tokio::fs::create_dir_all(dir).await
    {
        tracing::error!(%error, path = %dir.display(), "建不出记窗口状态的目录");
        return;
    }
    if let Err(error) = tokio::fs::write(&path, if value { "true" } else { "false" }).await {
        tracing::error!(%error, path = %path.display(), "记不下主人手改的窗口值");
    }
}

/// 交回自动：删掉顶掉的记录。
pub(crate) async fn clear_manual(data_dir: &Path) {
    if let Err(error) = tokio::fs::remove_file(manual_path(data_dir)).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::error!(%error, "删不掉主人手改的窗口值记录");
    }
}

/// 读上次自动写的值；没有或读不懂都当没有（第一次升上来，文件里的值按自动值算）。
pub(crate) fn load_last_applied(data_dir: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(last_applied_path(data_dir)).ok()?;
    match text.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// 记下这次自动写的值；写不进去只记日志，下一轮重试。
pub(crate) async fn store_last_applied(data_dir: &Path, value: bool) {
    let path = last_applied_path(data_dir);
    if let Some(dir) = path.parent()
        && let Err(error) = tokio::fs::create_dir_all(dir).await
    {
        tracing::error!(%error, path = %dir.display(), "建不出记窗口状态的目录");
        return;
    }
    if let Err(error) = tokio::fs::write(&path, if value { "true" } else { "false" }).await {
        tracing::error!(%error, path = %path.display(), "记不下窗口状态");
    }
}

/// 读 `tuwenel.toml` 现在写的 `allow_invalid_tls_certificates`；没有这行或值认不出算错。
pub(crate) async fn allow_invalid(path: &Path) -> anyhow::Result<bool> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("读取 {}", path.display()))?;
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix(ALLOW_INVALID) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        return match value.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => anyhow::bail!("{} 里 {ALLOW_INVALID} 的值认不出：{other}", path.display()),
        };
    }
    anyhow::bail!("{} 里没有 {ALLOW_INVALID}", path.display())
}

/// 把 `tuwenel.toml` 的 `allow_invalid_tls_certificates` 改成 `value`；已经是这个值返回 false（不动文件）。
/// 原地写，不换 inode（这份文件是单文件绑定挂载）；调用方在写之后让 Tuwunel 热加载。
pub(crate) async fn set_allow_invalid(path: &Path, value: bool) -> anyhow::Result<bool> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("读取 {}", path.display()))?;
    let mut out = String::with_capacity(text.len());
    let mut found = false;
    for line in text.split_inclusive('\n') {
        if !found && line.trim_start().starts_with(ALLOW_INVALID) && line.contains('=') {
            found = true;
            out.push_str(&format!("{ALLOW_INVALID} = {value}\n"));
        } else {
            out.push_str(line);
        }
    }
    anyhow::ensure!(found, "{} 里没有 {ALLOW_INVALID}", path.display());
    if out == text {
        return Ok(false);
    }
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .with_context(|| format!("写 {}", path.display()))?;
    file.write_all(out.as_bytes()).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn store(dir: &Path, peers: serde_json::Value) {
        std::fs::create_dir_all(dir.join("core")).unwrap();
        std::fs::write(
            dir.join("core/peers.json"),
            serde_json::json!({ "peers": peers }).to_string(),
        )
        .unwrap();
    }

    fn paired(pin_state: serde_json::Value, pin: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "server_name": "old.example",
            "endpoint_id": "a".repeat(52),
            "phase": "paired",
            "pin_state": pin_state,
            "peer_pin": pin,
        })
    }

    fn pin() -> serde_json::Value {
        serde_json::json!([{ "der": "MIIB", "spki_sha256": "00", "pinned_at_ms": 1 }])
    }

    #[test]
    fn pending_records_keep_the_window_until_it_expires() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            serde_json::json!({ "old.example": paired(serde_json::json!({"pending": {"since_version": "0.8.0"}}), serde_json::json!([])) }),
        );
        assert_eq!(
            window_state(Some(dir.path()), "0.9.0").unwrap(),
            WindowState {
                needed: true,
                expired: 0
            }
        );
        assert_eq!(
            window_state(Some(dir.path()), "0.10.0").unwrap(),
            WindowState {
                needed: false,
                expired: 1
            }
        );
    }

    #[test]
    fn pinned_records_and_unanswered_requests_do_not_need_the_window() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            serde_json::json!({
                "old.example": paired(serde_json::json!("pinned"), pin()),
                "late.example": { "phase": "awaiting_peer" },
            }),
        );
        assert_eq!(
            window_state(Some(dir.path()), "0.9.0").unwrap(),
            WindowState::default()
        );
    }

    /// 老记录没有 pin_state：按还没钉住算，窗口从这次升级起算（空起点按没到期）。
    #[test]
    fn legacy_records_keep_the_window_open() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            serde_json::json!({ "old.example": { "phase": "paired" } }),
        );
        assert_eq!(
            window_state(Some(dir.path()), "9.9.9").unwrap(),
            WindowState {
                needed: true,
                expired: 0
            }
        );
    }

    #[test]
    fn no_file_and_no_data_dir_need_no_window() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            window_state(Some(dir.path()), "0.9.0").unwrap(),
            WindowState::default()
        );
        assert_eq!(window_state(None, "0.9.0").unwrap(), WindowState::default());
    }

    #[test]
    fn a_broken_store_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("core")).unwrap();
        std::fs::write(dir.path().join("core/peers.json"), "not json").unwrap();
        assert!(window_state(Some(dir.path()), "0.9.0").is_err());
    }

    fn toml_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuwunel.toml");
        std::fs::write(
            &path,
            "[global]\n# comment\nallow_invalid_tls_certificates = true\nport = [8008]\n"
                .replace("allow_invalid_tls_certificates", ALLOW_INVALID),
        )
        .unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn setting_the_flag_rewrites_only_that_line_and_is_idempotent() {
        let (_dir, path) = toml_dir();
        assert!(set_allow_invalid(&path, false).await.unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("[global]\n# comment\n{ALLOW_INVALID} = false\nport = [8008]\n")
        );
        assert!(!set_allow_invalid(&path, false).await.unwrap());
        assert!(set_allow_invalid(&path, true).await.unwrap());
    }

    #[tokio::test]
    async fn a_config_without_the_flag_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuwunel.toml");
        std::fs::write(&path, "[global]\n").unwrap();
        assert!(set_allow_invalid(&path, false).await.is_err());
    }
}
