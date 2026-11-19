//! 假数据下的停止控件：发一条消息就算 Bot 开了一轮，点停止走完“在停 → 结果”。结果按发的那句话定，
//! 壳的三种结果才都有东西可测：带 [`REFUSE`] 的被拒，带 [`OFFLINE`] 的没送到，其余停成。

use std::time::Duration;

use wonton_protocol::{
    AgentAction, AgentChainBot, AgentChainRow, AgentChainStatus, BotRef, ChainSpend,
    ConfirmationOrigin, CoreError, TimelineAction,
};

use super::ClientCore;
use crate::agent_chains::room_stop::{self, Attempt};

const REFUSE: &str = "refuse";
const OFFLINE: &str = "offline";
/// “在停”留多久再给结果：UI 测试和截图来得及看到。
const STOPPING_SHOWN: Duration = Duration::from_millis(1500);

fn chain_id(body: &str) -> String {
    format!("ui-fake-turn:{body}")
}

fn outcome(chain: &str) -> Attempt {
    if chain.contains(REFUSE) {
        Attempt::Failed(CoreError::Forbidden {
            reason: "ui fake: not allowed".into(),
        })
    } else if chain.contains(OFFLINE) {
        Attempt::Uncertain(CoreError::TransportUnavailable {
            reason: "ui fake: offline".into(),
        })
    } else {
        Attempt::Stopped
    }
}

impl ClientCore {
    /// 发文字：这条时间线上开一条在跑的链，上一轮的换掉。
    pub(super) fn dispatch_ui_fake_timeline(&self, action: TimelineAction) {
        let (TimelineAction::SendText { target, body, .. }
        | TimelineAction::SendComposer { target, body, .. }) = action
        else {
            return;
        };
        let views = self.views();
        room_stop::forget_settled(&views.agent_stops, &target);
        views.agent_chains.send_modify(|view| {
            view.chains.retain(|row| {
                row.origin.room_id != target.room_id || row.origin.thread_id != target.thread_root
            });
            view.chains.push(AgentChainRow {
                chain: chain_id(&body),
                user: "@me:home.example".into(),
                origin: ConfirmationOrigin {
                    room_id: target.room_id.clone(),
                    thread_id: target.thread_root.clone(),
                },
                bots: vec![AgentChainBot {
                    bot: BotRef {
                        plugin_id: "app.example.hermes".into(),
                        bot: "owner".into(),
                    },
                    layer: 1,
                }],
                rounds: 1,
                spend: ChainSpend::Unknown,
                status: AgentChainStatus::Running,
                can_stop: true,
                live: true,
            });
        });
    }

    pub(super) fn dispatch_ui_fake_agent(&self, action: AgentAction) {
        let AgentAction::StopTurns { target } = action else {
            return;
        };
        let views = self.views();
        let Some(chains) = room_stop::begin(&views.agent_chains, &views.agent_stops, &target)
        else {
            return;
        };
        let (view, stops) = (views.agent_chains.clone(), views.agent_stops.clone());
        // 假数据入口没有作用域的运行时可借，结果另起线程晚一点给。
        std::thread::spawn(move || {
            std::thread::sleep(STOPPING_SHOWN);
            let attempt = chains.first().map_or(Attempt::Stopped, |id| outcome(id));
            let stopped = attempt == Attempt::Stopped;
            if stopped {
                view.send_modify(|view| view.chains.retain(|row| !chains.contains(&row.chain)));
            }
            room_stop::record(&stops, &target, attempt);
            if stopped {
                std::thread::sleep(room_stop::STOPPED_SHOWN);
                room_stop::expire_stopped(&stops, &target);
            }
        });
    }
}
