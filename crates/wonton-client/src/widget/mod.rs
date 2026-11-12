//! 系统入口的快照（ADR-0100）：核心在 App 进程里把聊天、默认 Agent、插件的投影写成 JSON 文件，桌面小组件、
//! 控制中心按钮和系统搜索只读这份文件，不跑核心。
//!
//! 不经 `Action`：写快照没有壳要发的操作，作用域里的任务跟着 ViewModel 自己写（同 ADR-0097 的“方法不进
//! `dispatch`”）。写法：
//! - 去抖：两次落盘至少隔 [`MIN_INTERVAL`]，期间的变化合并成最后一次。清空（没配对、被吊销、换服务器）不等，
//!   立刻写：别人的会话名和消息不能在桌面上多留。
//! - 原子：先写临时文件再改名，读的一方不会读到半个文件。
//! - 说不准的时候不写：文件停在上一次，入口如实显示“上次更新于”。

mod projection;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tokio::time::Instant;
use wonton_protocol::{
    AppearanceViewModel, ChatViewModel, HomeViewModel, OnboardingViewModel, PluginsViewModel,
    WIDGET_SNAPSHOT_PATH, WidgetAccount, WidgetSnapshot, WidgetSnapshotRead,
};

/// 两次落盘的最短间隔。同步一阵一阵地改房间列表，入口又看不出两秒内的差别。
pub(crate) const MIN_INTERVAL: Duration = Duration::from_secs(2);

/// 入口（iOS 的小组件扩展、Android 的小组件）读快照：路径和格式都由核心定，壳只给数据目录。
#[uniffi::export]
pub fn read_widget_snapshot(data_dir: String) -> WidgetSnapshotRead {
    let path = Path::new(&data_dir).join(WIDGET_SNAPSHOT_PATH);
    match std::fs::read(&path) {
        Ok(bytes) => WidgetSnapshotRead::from_json(Some(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            WidgetSnapshotRead::from_json(None)
        }
        Err(error) => WidgetSnapshotRead::Unreadable {
            detail: error.to_string(),
        },
    }
}

/// 快照文件，和“刚写下的快照”的通道（壳据此通知系统刷新小组件、更新快捷方式）。核心只有一份，换服务器不换。
#[derive(Clone)]
pub(crate) struct WidgetFile {
    path: Arc<PathBuf>,
    written: watch::Sender<Option<WidgetSnapshot>>,
}

impl WidgetFile {
    pub(crate) fn new(data_dir: &Path) -> Self {
        Self {
            path: Arc::new(data_dir.join(WIDGET_SNAPSHOT_PATH)),
            written: watch::Sender::new(None),
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<WidgetSnapshot>> {
        self.written.subscribe()
    }

    /// 换服务器：上一台的数据立刻清掉，新的那台的数据到了再写。
    pub(crate) fn switching(&self, appearance: &AppearanceViewModel) {
        self.write(projection::switching(appearance));
    }

    fn read(&self) -> Option<WidgetSnapshot> {
        match WidgetSnapshotRead::from_json(std::fs::read(&*self.path).ok().as_deref()) {
            WidgetSnapshotRead::Ready { snapshot } => Some(snapshot),
            WidgetSnapshotRead::Missing | WidgetSnapshotRead::Unreadable { .. } => None,
        }
    }

    /// 写不进去时记日志、返回 false：文件还是上一次的，入口照旧显示它的时间，下次变化再试。
    fn write(&self, mut snapshot: WidgetSnapshot) -> bool {
        snapshot.updated_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        match self.write_file(&snapshot) {
            Ok(()) => {
                self.written.send_replace(Some(snapshot));
                true
            }
            Err(error) => {
                tracing::warn!(%error, path = %self.path.display(), "could not write the widget snapshot");
                false
            }
        }
    }

    fn write_file(&self, snapshot: &WidgetSnapshot) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec(snapshot).map_err(std::io::Error::other)?;
        let partial = self.path.with_extension("json.partial");
        std::fs::write(&partial, json)?;
        std::fs::rename(&partial, &*self.path)
    }
}

/// 快照投影要看的 ViewModel。接在壳订阅的发送端上（换服务器也不换），任务跟着作用域起停。
pub(crate) struct Inputs {
    pub(crate) home: watch::Receiver<HomeViewModel>,
    pub(crate) chat: watch::Receiver<ChatViewModel>,
    pub(crate) onboarding: watch::Receiver<OnboardingViewModel>,
    pub(crate) plugins: watch::Receiver<PluginsViewModel>,
    pub(crate) appearance: watch::Receiver<AppearanceViewModel>,
    pub(crate) external: bool,
}

impl Inputs {
    /// 投影此刻的值，并把这些值记成看过。
    fn project(&mut self, previous: Option<&WidgetSnapshot>) -> Option<WidgetSnapshot> {
        let home = self.home.borrow_and_update();
        let chat = self.chat.borrow_and_update();
        let onboarding = self.onboarding.borrow_and_update();
        let plugins = self.plugins.borrow_and_update();
        let appearance = self.appearance.borrow_and_update();
        projection::project(
            &projection::Views {
                home: &home,
                chat: &chat,
                onboarding: &onboarding,
                plugins: &plugins,
                appearance: &appearance,
                external: self.external,
            },
            previous,
        )
    }

    /// 等任何一个变化；发送端没了（核心释放）时返回 false。
    async fn changed(&mut self) -> bool {
        tokio::select! {
            changed = self.home.changed() => changed.is_ok(),
            changed = self.chat.changed() => changed.is_ok(),
            changed = self.onboarding.changed() => changed.is_ok(),
            changed = self.plugins.changed() => changed.is_ok(),
            changed = self.appearance.changed() => changed.is_ok(),
        }
    }
}

/// 一个作用域里的写入任务：跟着 ViewModel 写，作用域停了它就停。
pub(crate) async fn run(file: WidgetFile, mut inputs: Inputs) {
    // 插件列表还没取到时沿用的那份：先是文件里上次写的。
    let mut previous = file.read();
    // 这个作用域里写过的；第一次说得准就写，“上次更新于”跟着刷新。
    let mut written: Option<WidgetSnapshot> = None;
    let mut written_at: Option<Instant> = None;
    loop {
        let next = inputs
            .project(previous.as_ref())
            .filter(|next| written.as_ref() != Some(next));
        if let Some(next) = next {
            let due = written_at
                .map(|at| at + MIN_INTERVAL)
                .filter(|due| next.account == WidgetAccount::SignedIn && Instant::now() < *due);
            if let Some(due) = due {
                tokio::select! {
                    () = tokio::time::sleep_until(due) => {}
                    alive = inputs.changed() => if !alive { return },
                }
                continue;
            }
            if file.write(next.clone()) {
                previous = Some(next.clone());
                written = Some(next);
                written_at = Some(Instant::now());
            }
        }
        if !inputs.changed().await {
            return;
        }
    }
}
