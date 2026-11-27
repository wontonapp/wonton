use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{
    BotAccess, BotChatUnavailable, CoreError, RoomBot, SharedBot, SharedBotsState, SharedBotsView,
};

use super::super::lock;
use super::Bots;
use super::Local;

impl Bots {
    /// 取当前人的 Bot 可达清单：单飞、按代号丢迟到结果；失败时留上一份，不清空。
    pub(super) fn refresh_shared(self: &Arc<Self>) {
        if self.session.borrow().is_none() {
            return;
        }
        let serial = {
            let mut local = lock(&self.local);
            local.shared_serial = self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1;
            local.shared_serial
        };
        let plugins = self.plugins.clone();
        let bots = self.clone();
        self.runtime.spawn(async move {
            let outcome = plugins.shared_bots().await;
            let mut local = lock(&bots.local);
            if !Self::apply_shared_result(&mut local, serial, outcome) {
                return;
            }
            drop(local);
            bots.publish_contacts();
            // 多个等待者（房间行、活动刷新）都该重算；notify_one 只叫醒一个。
            bots.changed.notify_waiters();
        });
    }

    /// 把一次清单结果写进本地。代号不匹配（换过人/换过会话）和取数失败都保留旧值；
    /// 旧核心的 404 是 `Ok(None)`，清空而不是当失败。整份不可信时行按 Unknown 用。
    fn apply_shared_result(
        local: &mut Local,
        serial: u64,
        outcome: Result<Option<SharedBotsView>, CoreError>,
    ) -> bool {
        if local.shared_serial != serial {
            return false;
        }
        match outcome {
            Ok(Some(view)) => local.shared = Some(Self::rows_from_view(view)),
            Ok(None) => local.shared = None,
            // 取不到就留上一份，界面别因为一次断线把能选的 Bot 全撤掉。
            Err(_) => return false,
        }
        true
    }

    /// 整份清单不可信时每一行都按 Unknown 用：壳照常能发，但不把上一轮的结论当本轮。
    fn rows_from_view(mut view: SharedBotsView) -> Vec<SharedBot> {
        if view.state != SharedBotsState::Ready {
            for row in &mut view.bots {
                // 缺记录是“已经知道”的事实，不随整份清单不可信一起抹掉。
                if matches!(
                    row.access,
                    BotAccess::Unusable {
                        reason: BotChatUnavailable::BotMissing
                    }
                ) {
                    continue;
                }
                row.access = BotAccess::Unknown;
            }
        }
        view.bots
    }

    /// 房间里的对方是 Bot 时给出身份和可用性：以核心的可达清单为准；外站 Bot、旧核心或清单还没到时退回活动快照，
    /// 退不出仍是 `None`（当普通人，输入照常）。
    pub(in crate::chat) fn room_bot(
        &self,
        user_id: Option<&str>,
        room_name: &str,
        name_fallback: bool,
    ) -> Option<RoomBot> {
        let shared = lock(&self.local);
        let row = user_id
            .and_then(|user_id| {
                shared
                    .shared
                    .as_ref()?
                    .iter()
                    .find(|row| row.user_id.as_deref() == Some(user_id))
            })
            .or_else(|| {
                if !name_fallback {
                    return None;
                }
                // 记录没建出来的行没有 Matrix ID，按插件显示名和房间名对上；同名时只当提示，不当发送资格。
                shared.shared.as_ref()?.iter().find(|row| {
                    row.user_id.is_none()
                        && matches!(
                            row.access,
                            BotAccess::Unusable {
                                reason: BotChatUnavailable::BotMissing
                            }
                        )
                        && row.title == room_name
                })
            });
        if let Some(row) = row {
            return Some(RoomBot {
                bot: row.bot.clone(),
                // 缺记录的行没有自己的 Matrix ID：用房间里一直记着的那一个，发消息/建私聊不空着。
                user_id: row
                    .user_id
                    .clone()
                    .unwrap_or_else(|| user_id.unwrap_or_default().to_owned()),
                title: row.title.clone(),
                access: row.access,
            });
        }
        drop(shared);
        let view = self.view.borrow();
        let bot = user_id
            .and_then(|user_id| view.bots.iter().find(|bot| bot.user_id == user_id))
            .or_else(|| {
                // 旧核心/联邦对不上 ID 时，只在私聊里按显示名找一次；找不到仍是普通人。
                name_fallback
                    .then(|| view.bots.iter().find(|bot| bot.name == room_name))
                    .flatten()
            })?;
        Some(RoomBot {
            bot: bot.bot.clone(),
            user_id: bot.user_id.clone(),
            title: bot.name.clone(),
            access: bot.availability.into(),
        })
    }
}

#[cfg(test)]
mod shared_rows_tests {
    use super::Local;

    fn row(access: BotAccess) -> SharedBot {
        SharedBot {
            bot: BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            user_id: Some("@bot:test".into()),
            title: "Bot".into(),
            icon: None,
            access,
        }
    }

    #[test]
    fn not_ready_downgrades_every_row_to_unknown() {
        let view = SharedBotsView {
            state: SharedBotsState::Unknown,
            bots: vec![
                row(BotAccess::Own),
                row(BotAccess::Unusable {
                    reason: wonton_protocol::BotChatUnavailable::AppNotShared,
                }),
            ],
        };
        let rows = Bots::rows_from_view(view);
        assert!(rows.iter().all(|row| row.access == BotAccess::Unknown));
    }

    #[test]
    fn failed_or_stale_results_keep_the_previous_list() {
        let mut local = Local {
            shared: Some(vec![row(BotAccess::Own)]),
            shared_serial: 2,
            ..Default::default()
        };
        assert!(!Bots::apply_shared_result(
            &mut local,
            1,
            Ok(Some(SharedBotsView {
                state: SharedBotsState::Ready,
                bots: vec![row(BotAccess::Shared)],
            })),
        ));
        assert_eq!(local.shared.as_ref().unwrap()[0].access, BotAccess::Own);
        assert!(!Bots::apply_shared_result(
            &mut local,
            2,
            Err(CoreError::Upstream {
                reason: "offline".into(),
            }),
        ));
        assert!(local.shared.is_some());
        assert!(Bots::apply_shared_result(&mut local, 2, Ok(None)));
        assert!(local.shared.is_none());
    }

    #[test]
    fn not_ready_result_writes_rows_as_unknown() {
        let mut local = Local {
            shared_serial: 7,
            ..Default::default()
        };
        assert!(Bots::apply_shared_result(
            &mut local,
            7,
            Ok(Some(SharedBotsView {
                state: SharedBotsState::Unknown,
                bots: vec![row(BotAccess::Shared)],
            })),
        ));
        assert_eq!(local.shared.unwrap()[0].access, BotAccess::Unknown);
    }

    #[test]
    fn ready_keeps_the_reasons() {
        let view = SharedBotsView {
            state: SharedBotsState::Ready,
            bots: vec![row(BotAccess::Shared)],
        };
        assert_eq!(Bots::rows_from_view(view)[0].access, BotAccess::Shared);
    }
}
