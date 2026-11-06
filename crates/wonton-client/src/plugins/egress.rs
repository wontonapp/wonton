//! 出网代理的设置页（ADR-0023 的“接口：App 和核心”）：取设置和探测结果，改设置前算出要不要主人确认重启。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use wonton_protocol::{
    CoreError, EgressConfirmation, EgressCurrent, EgressFailure, EgressPathState, EgressProbe,
    EgressProxySettings, EgressProxySource, EgressProxyUpdate, EgressRestart, PluginFailure,
    PluginProblem,
};

use super::api::{INSTALL_TIMEOUT, READ_TIMEOUT};
use super::{POLL_INTERVAL, PluginsLink};

const PATH: [&str; 1] = ["egress-proxy"];
/// 核心的探测结果超过 1 分钟算旧，这时它在后台再测，要等 5 秒以上。
const PROBE_FRESH_MS: u64 = 60_000;
/// 取到旧结果时最多再取这么多次。App 和服务器的时钟对不上时，结果永远显得旧，不能一直取下去。
const FOLLOW_UPS: usize = 8;

impl PluginsLink {
    /// `RefreshEgressProxy`：主人打开设置页。正在保存时等保存的结果。
    pub(crate) fn refresh_egress(self: &Arc<Self>) {
        if self.view.borrow().egress.saving {
            return;
        }
        self.view
            .send_modify(|view| view.egress.save_problem = None);
        self.follow_egress();
    }

    fn follow_egress(self: &Arc<Self>) {
        let generation = self.generation.load(Ordering::SeqCst);
        let epoch = self.egress_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.view.send_modify(|view| view.egress.loading = true);
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            for attempt in 0..=FOLLOW_UPS {
                if attempt > 0 {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
                let fetched = link.fetch_egress().await;
                if link.egress_epoch.load(Ordering::SeqCst) != epoch || !link.is_current(generation)
                {
                    return;
                }
                let settled = fetched.as_ref().map_or(true, probe_settled);
                link.view.send_modify(|view| {
                    view.egress.loading = false;
                    match fetched {
                        Ok(settings) => {
                            view.egress.current = Some(current_of(settings));
                            view.egress.load_problem = None;
                        }
                        Err(problem) => view.egress.load_problem = Some(problem),
                    }
                });
                if settled {
                    return;
                }
            }
        });
    }

    async fn fetch_egress(&self) -> Result<EgressProxySettings, PluginProblem> {
        self.api()?.get(&PATH, &[], READ_TIMEOUT).await
    }

    /// `SaveEgressProxy`：第一次配上或彻底取消、又有插件要重启时先等主人确认，其余的直接改。
    pub(crate) fn save_egress(
        self: &Arc<Self>,
        address: &str,
        direct_cidrs: &str,
        use_plugin: bool,
    ) -> Result<(), CoreError> {
        let (update, restarts) = {
            let view = self.view.borrow();
            if view.egress.saving {
                return Err(CoreError::NotReady {
                    reason: "上一次改动还没做完".to_owned(),
                });
            }
            let Some(current) = view.egress.current.as_ref() else {
                return Err(CoreError::NotReady {
                    reason: "还没取到出网代理的设置".to_owned(),
                });
            };
            planned(&current.settings, address, direct_cidrs, use_plugin)?
        };
        if restarts.is_empty() {
            self.put_egress(update);
        } else {
            self.view.send_modify(|view| {
                view.egress.confirmation = Some(EgressConfirmation {
                    restarts,
                    cancelling: !update.use_plugin && update.url.is_none(),
                    update,
                });
            });
        }
        Ok(())
    }

    /// `ConfirmEgressProxy`。
    pub(crate) fn confirm_egress(self: &Arc<Self>) -> Result<(), CoreError> {
        let mut confirmation = None;
        self.view
            .send_modify(|view| confirmation = view.egress.confirmation.take());
        let Some(confirmation) = confirmation else {
            return Err(CoreError::InvalidInput {
                reason: "没有等待确认的改动".to_owned(),
            });
        };
        self.put_egress(confirmation.update);
        Ok(())
    }

    pub(crate) fn dismiss_egress_confirmation(&self) {
        self.view
            .send_modify(|view| view.egress.confirmation = None);
    }

    fn put_egress(self: &Arc<Self>, update: EgressProxyUpdate) {
        let generation = self.generation.load(Ordering::SeqCst);
        self.egress_epoch.fetch_add(1, Ordering::SeqCst);
        self.view.send_modify(|view| {
            view.egress.saving = true;
            view.egress.loading = false;
            view.egress.save_problem = None;
        });
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let sent = match link.api() {
                Ok(api) => api.put(&PATH, &update, INSTALL_TIMEOUT).await,
                Err(problem) => Err(problem),
            };
            if !link.is_current(generation) {
                return;
            }
            match sent {
                Ok(settings) => {
                    let settled = probe_settled(&settings);
                    link.view.send_modify(|view| {
                        view.egress.saving = false;
                        view.egress.current = Some(current_of(settings));
                        view.egress.load_problem = None;
                    });
                    if !settled {
                        link.follow_egress();
                    }
                }
                Err(problem) => {
                    link.view.send_modify(|view| {
                        view.egress.saving = false;
                        view.egress.save_problem = Some(save_problem(problem));
                    });
                    // 要重启的插件可能变了（409 `changed`），下一次确认要用新的列表。
                    link.follow_egress();
                }
            }
            link.refresh_installed();
        });
    }
}

fn planned(
    settings: &EgressProxySettings,
    address: &str,
    direct_cidrs: &str,
    use_plugin: bool,
) -> Result<(EgressProxyUpdate, Vec<EgressRestart>), CoreError> {
    let address = address.trim();
    if settings.source == EgressProxySource::Env
        && (use_plugin || settings.url.as_deref().unwrap_or_default() != address)
    {
        return Err(CoreError::InvalidInput {
            reason: "代理由 WONTON_EGRESS_PROXY 指定，不能在这里改".to_owned(),
        });
    }
    if use_plugin && !address.is_empty() {
        return Err(CoreError::InvalidInput {
            reason: "使用代理插件时不能同时填写代理地址".to_owned(),
        });
    }
    if use_plugin && settings.provider.is_none() {
        return Err(CoreError::NotReady {
            reason: "还没有安装代理插件".to_owned(),
        });
    }
    let wanted = use_plugin || !address.is_empty();
    let restarts = if wanted == configured(settings) {
        Vec::new()
    } else {
        settings.restarts.clone()
    };
    let update = EgressProxyUpdate {
        url: (!address.is_empty()).then(|| address.to_owned()),
        direct_cidrs: direct_cidrs
            .split(|c: char| c == ',' || c == '，' || c.is_whitespace())
            .filter(|cidr| !cidr.is_empty())
            .map(str::to_owned)
            .collect(),
        restarts: restarts.iter().map(|plugin| plugin.id.clone()).collect(),
        use_plugin,
    };
    Ok((update, restarts))
}

/// 请求断开时核心照样做完（ADR-0023），这时不知道改好了没有。
fn save_problem(problem: PluginProblem) -> PluginProblem {
    if problem.reason != PluginFailure::TransportUnavailable {
        return problem;
    }
    PluginProblem {
        reason: problem.reason,
        message: format!("不确定改好了没有，连上后再看一下：{}", problem.message),
    }
}

fn current_of(settings: EgressProxySettings) -> EgressCurrent {
    let configured = configured(&settings);
    EgressCurrent {
        address: settings.url.clone().unwrap_or_default(),
        address_locked: settings.source == EgressProxySource::Env,
        use_plugin: settings.source == EgressProxySource::Plugin,
        direct_cidrs: settings.direct_cidrs.join(", "),
        plugin_path: configured.then(|| path_state(settings.plugin_path.as_ref())),
        core_path: configured.then(|| path_state(settings.core_path.as_ref())),
        settings,
    }
}

fn path_state(probe: Option<&EgressProbe>) -> EgressPathState {
    let Some(probe) = probe else {
        return EgressPathState::Probing;
    };
    let Some(problem) = &probe.problem else {
        return EgressPathState::Reachable;
    };
    let message = problem.message.clone();
    match problem.reason {
        EgressFailure::RelayFailed
        | EgressFailure::ProxyUnreachable
        | EgressFailure::ProxyRejected => EgressPathState::Failed { message },
        EgressFailure::UpstreamFailed | EgressFailure::Other => {
            EgressPathState::Uncertain { message }
        }
    }
}

/// 没配代理，或者手上的探测结果够新，不用接着取。
fn probe_settled(settings: &EgressProxySettings) -> bool {
    if !configured(settings) {
        return true;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0));
    settings
        .plugin_path
        .as_ref()
        .is_some_and(|probe| now.saturating_sub(probe.probed_at_ms) < PROBE_FRESH_MS)
}

fn configured(settings: &EgressProxySettings) -> bool {
    settings.source == EgressProxySource::Plugin || settings.url.is_some()
}
