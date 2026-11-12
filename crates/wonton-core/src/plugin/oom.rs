//! Docker OOM 事件的短期记录和插件提示（ADR-0037）。

use std::collections::BTreeMap;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::models::{EventMessage, EventMessageTypeEnum, HostConfig};
use futures_util::StreamExt;
use tokio::time::Instant;
use wonton_protocol::CardLine;

use super::host_check;
use super::manager::PluginManager;
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::render::LABEL_SERVICE;

const RECONNECT_AFTER: Duration = Duration::from_secs(5);
const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

/// 容器被杀时可能到顶的上限。OOM 事件不说是哪一层到顶：容器在共用的 slice 里时，服务自己没到上限也可能被杀。
#[derive(Debug, Clone, Copy)]
pub(super) struct OomLimit {
    memory: Option<u64>,
    shared_total: bool,
}

impl OomLimit {
    pub(super) fn of(host: Option<&HostConfig>) -> Self {
        Self {
            memory: host
                .and_then(|host| host.memory)
                .filter(|memory| *memory > 0)
                .map(|memory| memory as u64),
            shared_total: host
                .and_then(|host| host.cgroup_parent.as_deref())
                .is_some_and(|parent| !parent.is_empty()),
        }
    }
}

/// （插件，服务）→（时间，容器当时的上限）。
type OomEntries = BTreeMap<(PluginId, String), (Instant, OomLimit)>;

#[derive(Default)]
pub(super) struct OomRecords {
    records: Mutex<OomEntries>,
}

impl OomRecords {
    fn record(&self, plugin: PluginId, service: String, limit: OomLimit) {
        self.records
            .lock()
            .expect("OOM 记录锁中毒")
            .insert((plugin, service), (Instant::now(), limit));
    }

    fn clear(&self, plugin: &PluginId) -> bool {
        let mut records = self.records.lock().expect("OOM 记录锁中毒");
        let before = records.len();
        records.retain(|(id, _), _| id != plugin);
        records.len() != before
    }

    fn notices(&self, plugin: &PluginId) -> Vec<(String, OomLimit)> {
        let mut records = self.records.lock().expect("OOM 记录锁中毒");
        records.retain(|_, (at, _)| at.elapsed() < RETENTION);
        records
            .iter()
            .filter(|((id, _), _)| id == plugin)
            .map(|((_, service), (_, limit))| (service.clone(), *limit))
            .collect()
    }
}

fn limit_text(limit: OomLimit) -> String {
    let size = limit.memory.map(host_check::size);
    match (size, limit.shared_total) {
        (Some(size), true) => format!("（服务上限 {size}，也可能是所有插件加起来到了总上限）"),
        (Some(size), false) => format!("（上限 {size}）"),
        (None, true) => "（可能是所有插件加起来到了总上限）".to_owned(),
        (None, false) => String::new(),
    }
}

/// 插件还在运行时 `notices` 里的那一行。
fn oom_notice(service: &str, limit: OomLimit) -> String {
    format!("服务 {service} 内存不够，被系统结束过{}", limit_text(limit))
}

/// 容器因此不在运行时插件异常的原因。
pub(super) fn oom_reason(service: &str, limit: OomLimit) -> String {
    format!("服务 {service} 内存不够，被系统结束{}", limit_text(limit))
}

impl PluginManager {
    /// 订阅 Docker 的容器 OOM 事件，断开后重连。断开期间的事件丢了，主进程被杀的情况
    /// 还有 [`super::health::check_now`] 读 `State.OOMKilled`。
    pub fn spawn_oom_watch(self: &Arc<Self>) {
        let manager = Arc::downgrade(self);
        let engine = self.engine.clone();
        tokio::spawn(async move {
            loop {
                let mut events = pin!(engine.oom_events());
                let problem = loop {
                    match events.next().await {
                        Some(Ok(event)) => {
                            let Some(manager) = manager.upgrade() else {
                                return;
                            };
                            manager.handle_oom_event(event).await;
                        }
                        Some(Err(error)) => break error.to_string(),
                        None => break "事件流结束了".to_owned(),
                    }
                };
                tracing::warn!(%problem, "Docker 的 OOM 事件断开了，稍后重连");
                tokio::time::sleep(RECONNECT_AFTER).await;
            }
        });
    }

    async fn handle_oom_event(&self, event: EventMessage) {
        if event.typ != Some(EventMessageTypeEnum::CONTAINER)
            || event.action.as_deref() != Some("oom")
        {
            return;
        }
        let Some(container) = event.actor.and_then(|actor| actor.id) else {
            return;
        };
        let info = match self.engine.inspect_container(&container).await {
            Ok(Some(info)) => info,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(container, %error, "读取 OOM 容器失败");
                return;
            }
        };
        let Some(config) = info.config else {
            return;
        };
        let labels = config.labels.unwrap_or_default();
        if !self.cfg.instance.owns(&labels) {
            return;
        }
        let Some(plugin) = labels
            .get(LABEL_PLUGIN)
            .and_then(|value| PluginId::parse(value).ok())
        else {
            return;
        };
        let Some(service) = labels.get(LABEL_SERVICE).cloned() else {
            return;
        };
        let limit = OomLimit::of(info.host_config.as_ref());
        self.oom.record(plugin, service, limit);
        self.changed();
    }

    pub(super) fn oom_notices(&self, plugin: &PluginId) -> Vec<CardLine> {
        self.oom
            .notices(plugin)
            .into_iter()
            .map(|(service, limit)| CardLine {
                text: oom_notice(&service, limit),
                emphasis: false,
            })
            .collect()
    }

    pub(super) fn clear_oom(&self, plugin: &PluginId) {
        if self.oom.clear(plugin) {
            self.changed();
        }
    }
}
