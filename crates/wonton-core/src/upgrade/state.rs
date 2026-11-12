//! 状态文件 `<数据目录>/core/upgrade/state.json`：这次升级做到哪了。发起时核心写第一份，之后升级器写；只有确认升级器
//! 不在了，核心才接手写结果。新旧版本的核心和升级器都读它，字段只增不改，读不懂的阶段或结果按“不确定”算。

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use wonton_protocol::{
    CoreUpgradeFailure, CoreUpgradePhase, CoreUpgradeProblem, CoreUpgradeRun, CoreUpgradeRunState,
    CoreUpgradeTarget, CoreVersion,
};

/// `pulling` 的上限。
pub const PULL_LIMIT: Duration = Duration::from_secs(60 * 60);
/// `pulling` 里这么久没有进展算卡住。
pub const PULL_STALL: Duration = Duration::from_secs(5 * 60);
/// 从进入 `backing_up` 到 `starting` 结束的总时限。
pub const WINDOW_LIMIT: Duration = Duration::from_secs(30 * 60);
/// 过了健康检查后观察这么久。
pub const OBSERVE: Duration = Duration::from_secs(3 * 60);
/// `observing` 从第一次进入算起的上限：一次 `up` 最长 15 分钟，加观察 3 分钟。
pub const OBSERVING_LIMIT: Duration = Duration::from_secs(18 * 60);
/// `rolling_back` 的上限。
pub const ROLLBACK_LIMIT: Duration = Duration::from_secs(20 * 60);
/// 超过阶段上限这么久，核心才接手（ADR-0025 的“卡住和反复崩溃”）。
pub const WATCHDOG_MARGIN: Duration = Duration::from_secs(10 * 60);
/// 同一阶段升级器最多启动这么多次。
pub const MAX_UPDATER_STARTS: u32 = 3;

/// 升级涉及的路径。容器内外相同。
#[derive(Debug, Clone)]
pub struct Layout {
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Layout {
    pub fn core_dir(&self) -> PathBuf {
        self.data_dir.join("core")
    }

    pub fn upgrade_dir(&self) -> PathBuf {
        self.core_dir().join("upgrade")
    }

    pub fn state_path(&self) -> PathBuf {
        self.upgrade_dir().join("state.json")
    }

    pub fn backup_root(&self) -> PathBuf {
        self.upgrade_dir().join("backup")
    }

    /// 取消的标记带着升级编号：旧标记取消不了新升级。
    pub fn cancel_path(&self, id: &str) -> PathBuf {
        self.upgrade_dir().join(format!("cancel-{id}"))
    }

    pub fn env_path(&self) -> PathBuf {
        self.install_dir.join(".env")
    }
}

/// 升级涉及的一个核心镜像。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// compose 用的引用：`仓库@digest`。本机构建的旧镜像可能只有 tag。
    pub image: String,
    pub digest: Option<String>,
    pub version: Option<String>,
}

/// 升级的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    RolledBack,
    Failed,
    Canceled,
    RollbackFailed,
    Unresolved,
    Unknown,
    #[serde(other)]
    Other,
}

impl Outcome {
    /// 这份备份可能是退回迁移前状态的唯一办法，留到之后某次 `completed`。
    pub fn keeps_backup(self) -> bool {
        matches!(
            self,
            Self::Unresolved | Self::RollbackFailed | Self::Unknown | Self::Other
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeState {
    pub id: String,
    pub from: ImageRef,
    pub to: ImageRef,
    /// 这次升级会迁移的卷；不为空就是不可逆的版本。
    #[serde(default)]
    pub migrates: Vec<String>,
    pub started_at: u64,
    pub phase: CoreUpgradePhase,
    pub phase_started_at: u64,
    /// 第一次进入这些阶段的时间：时限记在这里，升级器重启不重新计时。
    #[serde(default)]
    pub window_started_at: Option<u64>,
    #[serde(default)]
    pub observing_started_at: Option<u64>,
    #[serde(default)]
    pub rolling_back_started_at: Option<u64>,
    /// 升级器在这个阶段第几次启动。
    #[serde(default)]
    pub updater_starts: u32,
    #[serde(default)]
    pub backup_dir: Option<PathBuf>,
    /// 回滚要用的系统服务镜像，`completed` 后留着。
    #[serde(default)]
    pub rollback_images: Vec<String>,
    /// 上一次升级留下的回滚镜像，这次 `completed` 后就用不到了。
    #[serde(default)]
    pub stale_images: Vec<String>,
    #[serde(default)]
    pub result: Option<Outcome>,
    /// 失败的原因。正在回滚时已经有了，结果要带上它。
    #[serde(default)]
    pub problem: Option<CoreUpgradeProblem>,
}

pub fn problem(reason: CoreUpgradeFailure, message: impl Into<String>) -> CoreUpgradeProblem {
    CoreUpgradeProblem {
        reason,
        message: message.into(),
    }
}

impl UpgradeState {
    /// 刚发起的一次升级，还在 `pulling`。
    pub fn new(id: String, from: ImageRef, to: ImageRef, migrates: Vec<String>, now: u64) -> Self {
        Self {
            id,
            from,
            to,
            migrates,
            started_at: now,
            phase: CoreUpgradePhase::Pulling,
            phase_started_at: now,
            window_started_at: None,
            observing_started_at: None,
            rolling_back_started_at: None,
            updater_starts: 0,
            backup_dir: None,
            rollback_images: Vec::new(),
            stale_images: Vec::new(),
            result: None,
            problem: None,
        }
    }

    pub fn is_ended(&self) -> bool {
        self.result.is_some()
    }

    /// 这次升级到了动手的阶段又还没结束：回滚只退备份里的东西，这时候动插件，回滚后记录和容器就对不上了。
    pub fn freezes_plugins(&self) -> bool {
        !self.is_ended() && self.phase != CoreUpgradePhase::Pulling
    }

    pub fn is_irreversible(&self) -> bool {
        !self.migrates.is_empty()
    }

    pub fn enter(&mut self, phase: CoreUpgradePhase, now: u64) {
        self.phase = phase;
        self.phase_started_at = now;
        self.updater_starts = 1;
        match phase {
            CoreUpgradePhase::BackingUp => {
                self.window_started_at.get_or_insert(now);
            }
            CoreUpgradePhase::Observing => {
                self.observing_started_at.get_or_insert(now);
            }
            CoreUpgradePhase::RollingBack => {
                self.rolling_back_started_at.get_or_insert(now);
            }
            _ => {}
        }
    }

    /// 这个阶段的上限落在什么时候。
    pub fn phase_limit_at(&self) -> u64 {
        let after = |at: Option<u64>, limit: Duration| {
            at.unwrap_or(self.phase_started_at) + limit.as_secs()
        };
        match self.phase {
            CoreUpgradePhase::Pulling => after(Some(self.phase_started_at), PULL_LIMIT),
            CoreUpgradePhase::BackingUp
            | CoreUpgradePhase::Switching
            | CoreUpgradePhase::Starting => after(self.window_started_at, WINDOW_LIMIT),
            CoreUpgradePhase::Observing => after(self.observing_started_at, OBSERVING_LIMIT),
            CoreUpgradePhase::RollingBack => after(self.rolling_back_started_at, ROLLBACK_LIMIT),
            // 新版本写的阶段：不知道它的上限，按整个升级最长的算。
            CoreUpgradePhase::Other => after(
                Some(self.phase_started_at),
                PULL_LIMIT + WINDOW_LIMIT + OBSERVING_LIMIT + ROLLBACK_LIMIT,
            ),
        }
    }

    /// 到回滚结束的最长剩余秒数（各阶段上限加余量）：连接断了以后，客户端最多等这么久才显示“不确定”。
    pub fn remaining_secs(&self, now: u64) -> u64 {
        let rest = match self.phase {
            CoreUpgradePhase::Pulling => WINDOW_LIMIT + OBSERVING_LIMIT + ROLLBACK_LIMIT,
            CoreUpgradePhase::BackingUp
            | CoreUpgradePhase::Switching
            | CoreUpgradePhase::Starting => OBSERVING_LIMIT + ROLLBACK_LIMIT,
            CoreUpgradePhase::Observing => ROLLBACK_LIMIT,
            CoreUpgradePhase::RollingBack | CoreUpgradePhase::Other => Duration::ZERO,
        };
        (self.phase_limit_at() + rest.as_secs() + WATCHDOG_MARGIN.as_secs()).saturating_sub(now)
    }

    /// 接口上的这次升级。
    pub fn run(&self, now: u64) -> CoreUpgradeRun {
        CoreUpgradeRun {
            id: self.id.clone(),
            from: CoreVersion {
                version: self.from.version.clone(),
                digest: self.from.digest.clone(),
            },
            to: CoreUpgradeTarget {
                // 发起时就检查过：目标总有 digest。没有时是别的版本写的，空串和任何 digest 都对不上，不会算成完成。
                digest: self.to.digest.clone().unwrap_or_default(),
                version: self.to.version.clone(),
            },
            state: self.run_state(now),
        }
    }

    fn run_state(&self, now: u64) -> CoreUpgradeRunState {
        let problem = || {
            self.problem
                .clone()
                .unwrap_or_else(|| problem(CoreUpgradeFailure::Other, "状态文件里没有写原因"))
        };
        // 这几种结果要主人动手。命令由核心写进原因，壳不拼。
        let stuck = || {
            let mut problem = problem();
            if let Some(digest) = &self.from.digest {
                problem.message.push_str(&format!(
                    "。要退回旧版本，在主机上再执行一次安装命令，加上 WONTON_CORE_PIN={digest}"
                ));
            }
            problem
        };
        match self.result {
            Some(Outcome::Completed) => CoreUpgradeRunState::Completed,
            Some(Outcome::Canceled) => CoreUpgradeRunState::Canceled,
            Some(Outcome::RolledBack) => CoreUpgradeRunState::RolledBack { problem: problem() },
            Some(Outcome::Failed) => CoreUpgradeRunState::Failed { problem: problem() },
            Some(Outcome::RollbackFailed) => {
                CoreUpgradeRunState::RollbackFailed { problem: stuck() }
            }
            Some(Outcome::Unresolved) => CoreUpgradeRunState::Unresolved { problem: stuck() },
            Some(Outcome::Unknown) => CoreUpgradeRunState::Unknown { problem: stuck() },
            Some(Outcome::Other) => CoreUpgradeRunState::Other,
            None if self.phase == CoreUpgradePhase::Other => CoreUpgradeRunState::Other,
            None => CoreUpgradeRunState::InProgress {
                phase: self.phase,
                remaining_secs: self.remaining_secs(now),
            },
        }
    }
}

/// 没有状态文件时是 `None`。
pub fn load(path: &Path) -> anyhow::Result<Option<UpgradeState>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("读取 {}", path.display())),
    };
    let state = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} 读不懂，可能是更新的版本写的", path.display()))?;
    Ok(Some(state))
}

pub fn save(path: &Path, state: &UpgradeState) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).context("序列化升级状态")?;
    write_atomic(path, &bytes, 0o600)
}

/// 写临时文件、fsync、改名，再 fsync 所在目录：断电后要么是旧内容，要么是新内容。
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("{} 没有上级目录", path.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("建立 {}", dir.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.tmp"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&temp)
        .with_context(|| format!("写入 {}", temp.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("写入 {}", temp.display()))?;
    drop(file);
    fs::rename(&temp, path).with_context(|| format!("写入 {}", path.display()))?;
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .with_context(|| format!("同步 {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(name: &str) -> ImageRef {
        ImageRef {
            image: format!("r@{name}"),
            digest: Some(name.into()),
            version: None,
        }
    }

    fn pulling() -> UpgradeState {
        UpgradeState::new("u1".into(), image("a"), image("b"), Vec::new(), 1000)
    }

    /// 要主人动手的结果带上退回到哪个版本；回滚成功的不用。
    #[test]
    fn stuck_results_say_which_digest_to_go_back_to() {
        let mut state = pulling();
        state.problem = Some(problem(CoreUpgradeFailure::Unhealthy, "起不来"));
        state.result = Some(Outcome::Unresolved);
        let CoreUpgradeRunState::Unresolved { problem: stuck } = state.run(1000).state else {
            panic!("应该是 unresolved");
        };
        assert!(stuck.message.contains("WONTON_CORE_PIN=a"), "{stuck:?}");

        state.result = Some(Outcome::RolledBack);
        let CoreUpgradeRunState::RolledBack { problem: back } = state.run(1000).state else {
            panic!("应该是 rolled_back");
        };
        assert!(!back.message.contains("WONTON_CORE_PIN"), "{back:?}");
    }

    /// 兜底的最长等待：从进入 `backing_up` 算起约 78 分钟（ADR-0025 的已定 6）。
    #[test]
    fn the_longest_wait_is_counted_from_the_first_entry_into_each_phase() {
        let mut state = pulling();
        state.enter(CoreUpgradePhase::BackingUp, 2000);
        assert_eq!(state.remaining_secs(2000), 78 * 60);

        state.enter(CoreUpgradePhase::Observing, 3000);
        assert_eq!(state.remaining_secs(3000), (18 + 20 + 10) * 60);
        state.enter(CoreUpgradePhase::Observing, 3500);
        assert_eq!(state.observing_started_at, Some(3000));

        state.enter(CoreUpgradePhase::RollingBack, 4000);
        assert_eq!(state.remaining_secs(4000), 30 * 60);
        assert_eq!(state.remaining_secs(u64::MAX / 2), 0);
    }

    #[test]
    fn plugins_are_frozen_from_backing_up_until_there_is_a_result() {
        let mut state = pulling();
        assert!(!state.freezes_plugins());
        state.enter(CoreUpgradePhase::BackingUp, 2000);
        assert!(state.freezes_plugins());
        state.result = Some(Outcome::Unknown);
        assert!(!state.freezes_plugins());
    }

    /// 更新的版本写的阶段和结果：解析不失败，也不算成功。
    #[test]
    fn a_phase_or_result_from_a_newer_version_reads_as_uncertain() {
        let mut value = serde_json::to_value(pulling()).unwrap();
        value["phase"] = "new_phase".into();
        value["new_field"] = 1.into();
        let state: UpgradeState = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(state.run(1000).state, CoreUpgradeRunState::Other);

        value["phase"] = "observing".into();
        value["result"] = "new_result".into();
        let state: UpgradeState = serde_json::from_value(value).unwrap();
        assert_eq!(state.run(1000).state, CoreUpgradeRunState::Other);
        assert!(state.is_ended());
    }

    #[test]
    fn the_file_is_written_whole_and_a_missing_file_is_no_upgrade() {
        let dir =
            std::env::temp_dir().join(format!("wonton-state-{}", crate::secret::random_token()));
        let path = dir.join("upgrade/state.json");
        assert_eq!(load(&path).unwrap(), None);
        save(&path, &pulling()).unwrap();
        assert_eq!(load(&path).unwrap(), Some(pulling()));
        fs::remove_dir_all(dir).unwrap();
    }
}
