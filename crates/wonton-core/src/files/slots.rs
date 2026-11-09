//! 按插件分的读取名额。读到的内容都在内存里，所以总数有上限；一个读得慢的插件只占自己的名额，别的插件的文件照常读。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, timeout_at};

use super::{Failure, FileFailure};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

/// 一个插件同时最多读这么多个。
pub(super) const EACH_PLUGIN: usize = 2;
/// 所有插件加起来最多读这么多个：比一个插件的多，读慢了几个插件才会挡住别的。
pub(super) const ALL_PLUGINS: usize = 6;
/// 等名额最多这么久。
pub const SLOT_WAIT: Duration = Duration::from_secs(10);
/// 占着名额的读取超过这么久还没结束，才算到插件头上。等名额等得久主要看排了多少个：一次要 24 张、每张 2 秒的相册也会等满
/// [`SLOT_WAIT`]，插件没有问题。
const SLOW_READ: Duration = Duration::from_secs(30);

pub struct PluginSlots {
    all: Arc<Semaphore>,
    plugins: Mutex<HashMap<PluginId, Arc<PluginReads>>>,
    slow_read: Duration,
}

struct PluginReads {
    room: Arc<Semaphore>,
    /// 正在读的那些是什么时候开始的。两个名额都拿到了才算开始：还在等总数的不是这个插件慢。
    started: Mutex<Vec<Instant>>,
}

/// 占着的名额，放下就还。
pub struct PluginSlot {
    reads: Arc<PluginReads>,
    started: Instant,
    _own: OwnedSemaphorePermit,
    _shared: OwnedSemaphorePermit,
}

impl Drop for PluginSlot {
    fn drop(&mut self) {
        let mut started = self.reads.started.lock().expect("读取名额表锁中毒");
        if let Some(at) = started.iter().position(|start| *start == self.started) {
            started.swap_remove(at);
        }
    }
}

impl PluginSlots {
    pub fn new() -> Self {
        Self::with_slow_read(SLOW_READ)
    }

    fn with_slow_read(slow_read: Duration) -> Self {
        Self {
            all: Arc::new(Semaphore::new(ALL_PLUGINS)),
            plugins: Mutex::default(),
            slow_read,
        }
    }

    /// 一有读取就算慢。
    #[cfg(test)]
    pub fn blaming_any_read() -> Self {
        Self::with_slow_read(Duration::ZERO)
    }

    /// 总数还剩多少。
    #[cfg(test)]
    pub fn free(&self) -> usize {
        self.all.available_permits()
    }

    /// 先等自己的再等总的：读得慢的插件排队时不占总数。等不到只说明忙，不归给谁；只有正在读的已经读了很久，才是这个插件的问题。
    pub async fn acquire(
        &self,
        plugin: &PluginId,
        wait: Duration,
        langs: &Languages,
    ) -> Result<PluginSlot, Failure> {
        let reads = {
            let mut plugins = self.plugins.lock().expect("读取名额表锁中毒");
            plugins
                .entry(plugin.clone())
                .or_insert_with(|| {
                    Arc::new(PluginReads {
                        room: Arc::new(Semaphore::new(EACH_PLUGIN)),
                        started: Mutex::default(),
                    })
                })
                .clone()
        };
        let deadline = Instant::now() + wait;
        let own = timeout_at(deadline, reads.room.clone().acquire_owned())
            .await
            .map_err(|_| self.no_room(plugin, &reads, langs))?
            .expect("信号量不会关闭");
        let shared = timeout_at(deadline, self.all.clone().acquire_owned())
            .await
            .map_err(|_| {
                Failure::new(
                    FileFailure::ServiceUnavailable,
                    l10n::text(langs, "files-plugin-slots-exhausted", &[]),
                )
            })?
            .expect("信号量不会关闭");
        let started = Instant::now();
        reads
            .started
            .lock()
            .expect("读取名额表锁中毒")
            .push(started);
        Ok(PluginSlot {
            reads,
            started,
            _own: own,
            _shared: shared,
        })
    }

    fn no_room(&self, plugin: &PluginId, reads: &PluginReads, langs: &Languages) -> Failure {
        let started = reads.started.lock().expect("读取名额表锁中毒");
        match started.iter().min() {
            Some(oldest) if oldest.elapsed() >= self.slow_read => Failure::new(
                FileFailure::PluginUnavailable,
                l10n::text(
                    langs,
                    "files-plugin-read-overrun",
                    &[
                        ("plugin", plugin.as_str().into()),
                        (
                            "seconds",
                            wonton_l10n::Arg::from(self.slow_read.as_secs() as i64),
                        ),
                    ],
                ),
            ),
            _ => Failure::new(
                FileFailure::ServiceUnavailable,
                l10n::text(
                    langs,
                    "files-slots-full",
                    &[("plugin", plugin.as_str().into())],
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_millis(30);

    fn langs() -> Languages {
        Languages::from_header(None)
    }

    fn plugin(n: usize) -> PluginId {
        PluginId::parse(&format!("app.example.p{n}")).unwrap()
    }

    /// 一次要很多张：排队等不到不是插件的问题。别的插件不受影响，还了名额接着读。
    #[tokio::test]
    async fn a_long_queue_is_busy_and_blames_no_plugin() {
        let slots = PluginSlots::new();
        let mut held = Vec::new();
        for _ in 0..EACH_PLUGIN {
            held.push(slots.acquire(&plugin(0), WAIT, &langs()).await.unwrap());
        }
        let busy = slots
            .acquire(&plugin(0), WAIT, &langs())
            .await
            .err()
            .unwrap();
        assert_eq!(busy.kind, FileFailure::ServiceUnavailable);
        assert!(slots.acquire(&plugin(1), WAIT, &langs()).await.is_ok());
        held.pop();
        assert!(slots.acquire(&plugin(0), WAIT, &langs()).await.is_ok());
    }

    /// 读了很久还没结束，才是这个插件的问题。读完了就不算。
    #[tokio::test]
    async fn only_a_read_that_has_taken_long_is_blamed_on_the_plugin() {
        let slots = PluginSlots::with_slow_read(WAIT);
        let mut held = Vec::new();
        for _ in 0..EACH_PLUGIN {
            held.push(slots.acquire(&plugin(0), WAIT, &langs()).await.unwrap());
        }
        let slow = slots
            .acquire(&plugin(0), WAIT, &langs())
            .await
            .err()
            .unwrap();
        assert_eq!(slow.kind, FileFailure::PluginUnavailable);
        assert!(slow.to_string().contains("app.example.p0"), "{slow}");
        held.clear();
        assert!(slots.reads_of(&plugin(0)).is_empty());
    }

    #[tokio::test]
    async fn when_all_slots_are_taken_nobody_in_particular_is_blamed() {
        let slots = PluginSlots::new();
        let mut held = Vec::new();
        for n in 0..ALL_PLUGINS {
            held.push(slots.acquire(&plugin(n), WAIT, &langs()).await.unwrap());
        }
        let busy = slots
            .acquire(&plugin(99), WAIT, &langs())
            .await
            .err()
            .unwrap();
        assert_eq!(busy.kind, FileFailure::ServiceUnavailable);
    }

    /// 总数满了，排在总数上的占着这个插件自己的名额，它一点都没开始读：后来的不能怪它。
    #[tokio::test]
    async fn waiting_for_the_shared_pool_does_not_count_as_reading() {
        let slots = Arc::new(PluginSlots::blaming_any_read());
        let mut held = Vec::new();
        for n in 0..ALL_PLUGINS {
            held.push(slots.acquire(&plugin(n), WAIT, &langs()).await.unwrap());
        }
        let long = Duration::from_secs(5);
        let mut waiting = Vec::new();
        for _ in 0..EACH_PLUGIN {
            let slots = slots.clone();
            waiting.push(tokio::spawn(async move {
                let _ = slots.acquire(&plugin(99), long, &langs()).await;
            }));
        }
        tokio::time::sleep(WAIT).await;
        let busy = slots
            .acquire(&plugin(99), WAIT, &langs())
            .await
            .err()
            .unwrap();
        assert_eq!(busy.kind, FileFailure::ServiceUnavailable);
        for task in waiting {
            task.abort();
        }
    }

    impl PluginSlots {
        fn reads_of(&self, plugin: &PluginId) -> Vec<Instant> {
            let plugins = self.plugins.lock().unwrap();
            plugins[plugin].started.lock().unwrap().clone()
        }
    }
}
