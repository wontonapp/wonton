//! App 锁（ADR-0102）：开关和退到后台后多久要求解锁，存在本机的一个设置文件（做法同 `push-method.json`），
//! 锁没锁在这里判定。壳只做平台那三件事：探测设备有没有锁屏、调系统验证、画锁定页。
//!
//! 锁定状态有三种来路：进程启动（开着就锁）、退到后台超过宽限、壳验证通过后 [`AppLockLink::unlock`]。宽限用
//! 一个定时器到点锁上，读状态时再按时刻算一遍：定时器所在的作用域随换服务器结束，算出来的结果不会因为它没跑而错。
//! 设置文件读坏了不禁用，报原因并把开关当成关，设置页能重设（失败即停）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_protocol::{AppLockTimeout, AppLockViewModel, CoreError};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const SETTINGS_FILE: &str = "app-lock.json";

/// 设置文件的格式。宽限存枚举，不存裸秒数：加档位时旧文件照样读得懂。
#[derive(Default, Serialize, Deserialize)]
struct Settings {
    enabled: bool,
    #[serde(default)]
    timeout: AppLockTimeout,
}

struct State {
    enabled: bool,
    timeout: AppLockTimeout,
    locked: bool,
    /// 退到后台的时刻，宽限从这一刻起算；在前台时为空。
    backgrounded_at: Option<Instant>,
    problem: Option<String>,
    /// 每次前后台切换加一，定时器只在自己那一代还作数时动状态。
    epoch: u64,
    /// 宽限到点的定时器。换作用域会连运行时一起停掉，所以靠 `epoch` 和时刻判定兜底。
    timer: Option<JoinHandle<()>>,
}

impl State {
    fn due(&self, now: Instant) -> bool {
        self.enabled
            && self
                .backgrounded_at
                .is_some_and(|at| now.duration_since(at) >= self.timeout.span())
    }

    fn view(&self, now: Instant) -> AppLockViewModel {
        AppLockViewModel {
            enabled: self.enabled,
            timeout: self.timeout,
            locked: self.locked || self.due(now),
            problem: self.problem.clone(),
        }
    }
}

struct Inner {
    path: PathBuf,
    state: Mutex<State>,
    view: watch::Sender<AppLockViewModel>,
    /// 写文件和发布新值是一步，否则并发时文件和状态可能停在不同的设置上。
    writing: Mutex<()>,
}

/// 整机一个，跟着核心走，不随服务器切换重建。
#[derive(Clone)]
pub(crate) struct AppLockLink(Arc<Inner>);

impl AppLockLink {
    pub(crate) fn new(data_dir: &Path) -> Self {
        let path = data_dir.join(SETTINGS_FILE);
        let (settings, problem) = match read_settings(&path) {
            Ok(settings) => (settings.unwrap_or_default(), None),
            Err(reason) => {
                tracing::warn!("failed to read the app lock settings, using the default: {reason}");
                (Settings::default(), Some(reason))
            }
        };
        let state = State {
            enabled: settings.enabled,
            timeout: settings.timeout,
            // 启动必锁：进程能起来就是主人（或别人）刚打开 App。
            locked: settings.enabled,
            backgrounded_at: None,
            problem,
            epoch: 0,
            timer: None,
        };
        Self(Arc::new(Inner {
            path,
            view: watch::Sender::new(state.view(Instant::now())),
            state: Mutex::new(state),
            writing: Mutex::new(()),
        }))
    }

    pub(crate) fn view(&self) -> AppLockViewModel {
        self.state().view(Instant::now())
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<AppLockViewModel> {
        self.0.view.subscribe()
    }

    /// 此刻锁没锁。通知走这里而不是订阅，定时器没跑到也按时刻算得准。
    pub(crate) fn is_locked(&self) -> bool {
        let now = Instant::now();
        let state = self.state();
        state.locked || state.due(now)
    }

    /// 写不了时返回 `Storage`，设置不变。开或关都要落盘成功才生效。
    pub(crate) fn set(&self, enabled: bool, timeout: AppLockTimeout) -> Result<(), CoreError> {
        let _writing = self.lock_writing();
        let text = serde_json::to_vec(&Settings { enabled, timeout })
            .expect("the app lock settings are plain values and always serialize");
        write_private(&self.0.path, &text).map_err(|error| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "app-lock-write-failed",
                path = self.0.path.display().to_string(),
                reason = error.to_string()
            ),
        })?;
        let now = Instant::now();
        let mut state = self.state();
        state.enabled = enabled;
        state.timeout = timeout;
        state.problem = None;
        state.epoch += 1;
        // 关掉就放行；开着时主人正在设置页里，下一次进后台才锁。宽限改成短于已经过去的时间就当场锁上。
        if !enabled {
            state.locked = false;
            state.backgrounded_at = None;
        } else {
            state.locked |= state.due(now);
        }
        Self::publish(&state, &self.0.view, now);
        Ok(())
    }

    /// 壳的系统验证通过后调用。幂等。
    pub(crate) fn unlock(&self) {
        let now = Instant::now();
        let mut state = self.state();
        if !state.locked && !state.due(now) {
            return;
        }
        state.locked = false;
        // 解锁之后重新起算宽限：验证花的时间不算在后台里。
        state.backgrounded_at = None;
        state.epoch += 1;
        Self::publish(&state, &self.0.view, now);
    }

    /// App 进后台：开始算宽限，立即档当场锁上，其余档到点由定时器锁。`handle` 是当前作用域的运行时，
    /// 换服务器把它停掉时定时器一起没，回到前台按时刻算的结果照旧准。
    pub(crate) fn backgrounded(&self, handle: Option<tokio::runtime::Handle>) {
        let now = Instant::now();
        let mut state = self.state();
        if !state.enabled {
            return;
        }
        state.epoch += 1;
        let epoch = state.epoch;
        state.backgrounded_at = Some(now);
        state.locked |= state.timeout == AppLockTimeout::Immediate;
        Self::publish(&state, &self.0.view, now);
        if state.locked {
            return;
        }
        if let Some(timer) = state.timer.take() {
            timer.abort();
        }
        let timeout = state.timeout.span();
        drop(state);
        let Some(handle) = handle else { return };
        let link = self.clone();
        let timer = handle.spawn(async move {
            tokio::time::sleep(timeout).await;
            link.lock_if_due(epoch);
        });
        self.state().timer = Some(timer);
    }

    /// App 回到前台：宽限已经过去的当场锁上；还没过去的放行，宽限重新从下一次退到后台算。
    pub(crate) fn foregrounded(&self) {
        let now = Instant::now();
        let mut state = self.state();
        if !state.enabled {
            return;
        }
        state.epoch += 1;
        if let Some(timer) = state.timer.take() {
            timer.abort();
        }
        state.locked |= state.due(now);
        state.backgrounded_at = None;
        Self::publish(&state, &self.0.view, now);
    }

    /// 测试用：把退到后台的时刻往前挪 `elapsed`，再走到点判定，不真的等一分。没退过后台的先当成刚退。
    pub(crate) fn backdate_for_test(&self, elapsed: std::time::Duration) {
        let mut state = self.state();
        let at = state.backgrounded_at.unwrap_or_else(Instant::now);
        state.backgrounded_at = Some(at - elapsed);
        let epoch = state.epoch;
        drop(state);
        self.lock_if_due(epoch);
    }

    fn lock_if_due(&self, epoch: u64) {
        let now = Instant::now();
        let mut state = self.state();
        if state.epoch != epoch || !state.due(now) {
            return;
        }
        state.locked = true;
        state.backgrounded_at = None;
        Self::publish(&state, &self.0.view, now);
    }

    fn publish(state: &State, view: &watch::Sender<AppLockViewModel>, now: Instant) {
        view.send_if_modified(|current| {
            let next = state.view(now);
            let changed = *current != next;
            *current = next;
            changed
        });
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_writing(&self) -> MutexGuard<'_, ()> {
        self.0
            .writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 没设过时是 `None`。
fn read_settings(path: &Path) -> Result<Option<Settings>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(l10n!(
                CATALOG,
                language(),
                "app-lock-read-failed",
                path = path.display().to_string(),
                reason = error.to_string()
            ));
        }
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        l10n!(
            CATALOG,
            language(),
            "app-lock-corrupt",
            path = path.display().to_string(),
            reason = error.to_string()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 测试里不真的等宽限：把退到后台的时刻往前挪，再走一次到点判定。
    fn backdate(link: &AppLockLink, elapsed: Duration) {
        let mut state = link.state();
        state.backgrounded_at = Some(Instant::now() - elapsed);
        let epoch = state.epoch;
        drop(state);
        link.lock_if_due(epoch);
    }

    #[test]
    fn a_missing_file_starts_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        let view = link.view();
        assert!(!view.enabled);
        assert!(!view.locked);
        assert_eq!(view.timeout, AppLockTimeout::Immediate);
        assert!(view.problem.is_none());
    }

    #[test]
    fn a_cold_start_with_the_lock_on_is_locked() {
        let dir = tempfile::tempdir().unwrap();
        AppLockLink::new(dir.path())
            .set(true, AppLockTimeout::FiveMinutes)
            .unwrap();
        let reopened = AppLockLink::new(dir.path());
        assert!(reopened.view().locked);
        assert!(reopened.is_locked());
    }

    #[test]
    fn a_damaged_file_reports_and_stays_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), b"{").unwrap();
        let link = AppLockLink::new(dir.path());
        let view = link.view();
        assert!(!view.enabled);
        assert!(!view.locked);
        assert!(view.problem.is_some());
    }

    #[test]
    fn the_immediate_timeout_locks_on_the_way_to_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::Immediate).unwrap();
        link.unlock();
        link.backgrounded(None);
        assert!(link.view().locked);
    }

    #[test]
    fn no_background_no_lock() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::OneMinute).unwrap();
        link.unlock();
        link.foregrounded();
        assert!(!link.view().locked);
    }

    #[test]
    fn the_grace_expires_after_the_chosen_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::OneMinute).unwrap();
        link.unlock();
        link.backgrounded(None);
        backdate(&link, Duration::from_secs(30));
        assert!(!link.is_locked(), "还没到一分钟");
        backdate(&link, Duration::from_secs(61));
        assert!(link.is_locked());
    }

    #[test]
    fn coming_back_within_the_grace_stays_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::FiveMinutes).unwrap();
        link.unlock();
        link.backgrounded(None);
        backdate(&link, Duration::from_secs(120));
        link.foregrounded();
        assert!(!link.view().locked);
    }

    #[test]
    fn coming_back_after_the_grace_locks() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::OneMinute).unwrap();
        link.unlock();
        link.backgrounded(None);
        backdate(&link, Duration::from_secs(61));
        link.foregrounded();
        assert!(link.view().locked);
    }

    #[test]
    fn unlocking_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::Immediate).unwrap();
        link.backgrounded(None);
        assert!(link.view().locked);
        link.unlock();
        link.unlock();
        assert!(!link.view().locked);
    }

    #[test]
    fn turning_the_lock_off_releases_and_turning_it_on_does_not_lock_under_the_finger() {
        let dir = tempfile::tempdir().unwrap();
        let link = AppLockLink::new(dir.path());
        link.set(true, AppLockTimeout::Immediate).unwrap();
        assert!(!link.view().locked, "主人就在设置页里");
        link.backgrounded(None);
        assert!(link.view().locked);
        link.set(false, AppLockTimeout::Immediate).unwrap();
        assert!(!link.view().locked);
    }

    #[test]
    fn a_failed_write_keeps_the_old_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(SETTINGS_FILE)).unwrap();
        let link = AppLockLink::new(dir.path());
        let before = link.view();
        assert!(matches!(
            link.set(true, AppLockTimeout::FiveMinutes),
            Err(CoreError::Storage { .. })
        ));
        assert_eq!(link.view(), before);
    }
}
