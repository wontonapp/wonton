//! 敏感调用的确认和执行（ADR-0016）：发卡片、等回答，同意了再执行，结果写进审计和卡片。

use std::future::Future;
use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::Value;
use wonton_protocol::{ConfirmationState, SecretValues};

use super::audience::Subject;
use super::audit::{Basis, Decision, Grounds};
use super::chain_card::{Bound, bind_card};
use super::{Bus, Caller, Draft, Outcome, Refusal, Ticket, with_confirmation, with_failure};
use crate::bot_activity::Guard;

impl Bus {
    /// 请主人确认，确认了再执行 `run`。整个过程在后台任务里跑：Agent 等不及断开了，确认照样有效到过期，主人确认后
    /// 照样执行，结果在卡片上（ADR-0016）。
    pub(super) async fn confirmed<F, Fut>(
        self: &Arc<Self>,
        caller: &Caller,
        name: &str,
        args: Value,
        (draft, activity): (Draft, Guard),
        run: F,
    ) -> CallToolResult
    where
        F: FnOnce(Arc<Bus>, SecretValues, wonton_protocol::SettingValues, Vec<String>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Outcome> + Send,
    {
        self.confirmed_after_hold(
            caller,
            name,
            args,
            (draft, activity),
            run,
            (None, Subject::PluginAdmin),
        )
        .await
    }

    /// `held_ms`：发卡片之前先停住了多久（“我来操作”），写进审计。`subject` 定卡片发给谁（[`audience`]）。
    pub(super) async fn confirmed_after_hold<F, Fut>(
        self: &Arc<Self>,
        caller: &Caller,
        name: &str,
        args: Value,
        (draft, activity): (Draft, Guard),
        run: F,
        (held_ms, subject): (Option<u64>, Subject),
    ) -> CallToolResult
    where
        F: FnOnce(Arc<Bus>, SecretValues, wonton_protocol::SettingValues, Vec<String>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Outcome> + Send,
    {
        drop(activity);
        let bus = self.clone();
        let caller = caller.clone();
        let name = name.to_owned();
        let task = tokio::spawn(async move {
            let decision = bus.ask_for(&caller, subject, draft).await;
            match decision {
                Ok(ticket) => {
                    let activity = bus.plugins.activity.start_tool(&caller.id);
                    let outcome = run(
                        bus.clone(),
                        ticket.secrets.clone(),
                        ticket.settings.clone(),
                        ticket.devices.clone(),
                    )
                    .await;
                    drop(activity);
                    // `window` 有值：不是这次调用自己的卡片，是主人先前批准的放行时段。
                    let confirmation = if ticket.window.is_some() {
                        "window"
                    } else {
                        "card"
                    };
                    bus.record(
                        &caller,
                        &name,
                        &args,
                        Decision::Approved,
                        Some(Grounds {
                            basis: ticket.window.clone().map_or(Basis::Card, Basis::Window),
                            held_ms,
                        }),
                        &outcome,
                    );
                    bus.confirmations.finish(ticket, outcome.state).await;
                    with_confirmation(outcome.result, confirmation)
                }
                Err(refusal) => bus.refused(&caller, &name, &args, &refusal, held_ms),
            }
        });
        task.await
            .unwrap_or_else(|e| with_failure(&format!("确认流程意外中止：{e}"), "internal_error"))
    }

    /// 最高级的安装和更新（ADR-0036）：第一张批准后，再请主人点一张“我知道这个插件能控制整台主机”
    /// 的卡片，两张都同意才执行。`second` 是第二张的内容；没有最高级权限时是一次普通确认。
    /// 两张卡片都没有放宽选项：安装和更新不进放行时段（ADR-0024 对它们不适用）。
    pub(super) async fn confirmed_twice<F, Fut>(
        self: &Arc<Self>,
        caller: &Caller,
        name: &str,
        args: Value,
        (draft, second, device_second, held_ms, activity): (
            Draft,
            Option<Draft>,
            Option<Draft>,
            Option<u64>,
            Guard,
        ),
        run: F,
    ) -> CallToolResult
    where
        F: FnOnce(Arc<Bus>, SecretValues, wonton_protocol::SettingValues, Vec<String>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Outcome> + Send,
    {
        if second.is_none() && device_second.is_none() {
            return self
                .confirmed_after_hold(
                    caller,
                    name,
                    args,
                    (draft, activity),
                    run,
                    (held_ms, Subject::PluginAdmin),
                )
                .await;
        }
        let nvme = nvme_devices(&draft);
        drop(activity);
        let bus = self.clone();
        let caller = caller.clone();
        let name = name.to_owned();
        let task = tokio::spawn(async move {
            // 第二张卡片沿用第一张创建时冻结的管理员名单。
            let audience = bus.audience(&caller, Subject::PluginAdmin).await;
            let ticket = match bus.ask_for(&caller, Subject::PluginAdmin, draft).await {
                Ok(ticket) => ticket,
                Err(refusal) => return bus.refused(&caller, &name, &args, &refusal, None),
            };
            let second = pick_second(&bus, &caller, &nvme, second, device_second, &ticket);
            let second_ticket = if let Some(second) = second {
                match ask_second(&bus, &caller, second, audience).await {
                    Ok(second) => Some(second),
                    Err(refusal) => {
                        let message = refusal.message();
                        bus.confirmations
                            .finish(ticket, ConfirmationState::Failed { message })
                            .await;
                        return bus.refused(&caller, &name, &args, &refusal, None);
                    }
                }
            } else {
                None
            };
            let activity = bus.plugins.activity.start_tool(&caller.id);
            let outcome = run(
                bus.clone(),
                ticket.secrets.clone(),
                ticket.settings.clone(),
                ticket.devices.clone(),
            )
            .await;
            drop(activity);
            let confirmation = if ticket.window.is_some() {
                "window"
            } else {
                "card"
            };
            bus.record(
                &caller,
                &name,
                &args,
                Decision::Approved,
                Some(Grounds {
                    basis: Basis::Card,
                    held_ms,
                }),
                &outcome,
            );
            let state = outcome.state.clone();
            bus.confirmations.finish(ticket, state.clone()).await;
            if let Some(second_ticket) = second_ticket {
                bus.confirmations.finish(second_ticket, state).await;
            }
            with_confirmation(outcome.result, confirmation)
        });
        task.await
            .unwrap_or_else(|e| with_failure(&format!("确认流程意外中止：{e}"), "internal_error"))
    }
}

/// 最高级权限的第二张卡片：开出来、认链、等回答。出错时由调用方把第一张落成失败。
async fn ask_second(
    bus: &Arc<Bus>,
    caller: &Caller,
    second: Draft,
    audience: super::Audience,
) -> Result<Ticket, Refusal> {
    let (card, answered) = bus.confirmations.open_as(&caller.id, second, audience)?;
    // 停链抢在开卡片和绑卡片之间时自己撤回；第一张也一起落成取消。
    if bind_card(bus, caller, None, &card.id, "工具卡片").outcome == Bound::Withdrawn {
        return Err(Refusal::Cancelled);
    }
    bus.confirmations.wait(card, answered).await
}

/// 第二张卡片的内容：`second` 优先；第一张选了 NVMe 盘时改用盘的风险卡片，并加一句“能控制整台主机”的警语。
fn pick_second(
    bus: &Bus,
    caller: &Caller,
    nvme: &[(String, bool)],
    second: Option<Draft>,
    device_second: Option<Draft>,
    ticket: &Ticket,
) -> Option<Draft> {
    let selected_nvme = nvme
        .iter()
        .any(|(id, chosen)| ticket.devices.contains(id) || (ticket.devices.is_empty() && *chosen));
    let mut second = second.or_else(|| selected_nvme.then_some(device_second).flatten());
    if selected_nvme && let Some(second) = &mut second {
        super::cards::mark_nvme_risk(second, &bus.card_languages(caller, Subject::PluginAdmin));
    }
    second
}

/// 第一张卡片上勾了的 NVMe 盘：(设备 ID, 是否默认勾选)。
fn nvme_devices(draft: &Draft) -> Vec<(String, bool)> {
    draft
        .devices
        .iter()
        .filter(|device| {
            device.kind == wonton_protocol::PluginDeviceKind::Disk
                && device.target.starts_with("/dev/nvme")
        })
        .map(|device| (device.id.clone(), device.chosen))
        .collect()
}
