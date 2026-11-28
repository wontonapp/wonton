#[cfg(test)]
use std::pin::Pin;
use std::sync::Arc;

use wonton_matrix::MatrixSession;

use super::super::lock;
use super::Bots;

#[derive(Default)]
pub(in crate::chat) struct OwnEpoch {
    epoch: u64,
    retired: bool,
    // 同步生命周期没有 await；测试在捕获代号后、检查写槽资格前暂停一次真实调用。
    #[cfg(test)]
    pub(in crate::chat) before_refresh: Option<Box<dyn FnOnce() + Send>>,
}

/// 测试接缝的取数闭包：每次调用返回一次真实取数，结果原样交给发布闸门。
#[cfg(test)]
pub(in crate::chat) type OwnBotFetch = Arc<
    dyn Fn() -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            wonton_protocol::OwnPrivateBotView,
                            wonton_protocol::OwnBotProblem,
                        >,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

impl OwnEpoch {
    #[cfg(test)]
    pub(in crate::chat) fn current(&self) -> u64 {
        self.epoch
    }

    /// 领取有效代；退役后不再发号。
    pub(super) fn issue(&mut self) -> Option<u64> {
        if self.retired {
            return None;
        }
        self.epoch = self.epoch.wrapping_add(1);
        Some(self.epoch)
    }

    /// 只有当前且未退役的代能写槽。
    fn accepts(&self, epoch: u64) -> bool {
        !self.retired && self.epoch == epoch
    }
}

impl Bots {
    /// 不可逆退役：此后本链接不再发有效代，迟到的结果与清槽都被拒。
    pub(in crate::chat) fn retire_own_private_bot(&self) {
        let mut own = lock(&self.own_epoch);
        own.epoch = own.epoch.wrapping_add(1);
        own.retired = true;
    }
}

impl Bots {
    /// 本人的私人 Bot 身份是独立读取：成功写身份槽并清掉问题，失败只写本人问题槽，不占全局 `problem`。
    /// 有效性比对和写入在同一把 `own_epoch` 锁里，换会话或注销后迟到的结果不写回旧身份。
    pub fn refresh_own_private_bot(
        self: &Arc<Self>,
        session: Option<Arc<MatrixSession>>,
        epoch: u64,
    ) {
        // 接缝在暂停前快照：暂停期间装上或换掉的闭包不影响这次调用已经选定的分派。
        #[cfg(test)]
        let fetch = lock(&self.own_bot_fetch).clone();
        #[cfg(test)]
        {
            let pause = lock(&self.own_epoch).before_refresh.take();
            if let Some(pause) = pause {
                pause();
            }
        }
        #[cfg(test)]
        if let Some(fetch) = fetch {
            self.spawn_own_bot_fetch(fetch, epoch);
            return;
        }
        let Some(session) = session else {
            let own = lock(&self.own_epoch);
            if !own.accepts(epoch) {
                return;
            }
            self.view.send_modify(|view| {
                view.own_private_bot = None;
                view.own_private_bot_problem = None;
            });
            return;
        };
        let weak = Arc::downgrade(self);
        let plugins = self.plugins.clone();
        self.runtime.spawn(async move {
            let outcome = plugins.own_private_bot(&session).await;
            let Some(bots) = weak.upgrade() else {
                return;
            };
            bots.publish_own_private_bot(epoch, outcome);
        });
    }

    /// 接缝取数的后台任务：和会话取数是同一条发布闸门，闭包替代了会话上的取数。
    /// 一次性取走完成回执，在发布闸门判定完才发：用例据此确认旧任务已经过闸门，不等固定时长。
    #[cfg(test)]
    fn spawn_own_bot_fetch(self: &Arc<Self>, fetch: OwnBotFetch, epoch: u64) {
        let weak = Arc::downgrade(self);
        let done = lock(&self.own_bot_fetch_done).take();
        self.runtime.spawn(async move {
            let outcome = fetch().await;
            let Some(bots) = weak.upgrade() else {
                return;
            };
            bots.publish_own_private_bot(epoch, outcome);
            if let Some(done) = done {
                let _ = done.send(());
            }
        });
    }

    /// 结果发布：代比对与双槽写入在同一把锁里，换会话／注销后迟到的结果不写回旧身份。
    pub(in crate::chat) fn publish_own_private_bot(
        &self,
        epoch: u64,
        outcome: Result<wonton_protocol::OwnPrivateBotView, wonton_protocol::OwnBotProblem>,
    ) {
        let own = lock(&self.own_epoch);
        if !own.accepts(epoch) {
            return;
        }
        self.view.send_modify(|view| match outcome {
            Ok(own) => {
                view.own_private_bot = Some(own);
                view.own_private_bot_problem = None;
            }
            Err(problem) => view.own_private_bot_problem = Some(problem),
        });
        drop(own);
        self.publish_contacts();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_epoch_stops_issuing_and_rejects_late_results() {
        let mut own = OwnEpoch::default();
        let first = own.issue().unwrap();
        let second = own.issue().unwrap();
        assert_eq!(own.current(), second);
        assert!(!own.accepts(first));
        own.retired = true;
        let retired = own.current();
        assert_eq!(own.issue(), None);
        assert_eq!(own.current(), retired);
        assert!(!own.accepts(second));
    }
}
