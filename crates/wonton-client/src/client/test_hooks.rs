//! 只给测试用的钩子接口（feature `test-hooks`，生产构建没有）：打开、等报到、放行。
//! 钩子挂在当前作用域上（`Scope::open` 建的），换服务器会换作用域，测试不换服务器就一直是同一个。
//! 状态和等放行的机制在 [`crate::test_hooks`]。

use std::sync::Arc;
use std::time::Duration;

use super::ClientCore;

impl ClientCore {
    /// 打开卡片成功的钩子：下一次成功在“补不补取”之前先报到，等 [`Self::test_release_card_success`]
    /// 放行才继续。
    #[doc(hidden)]
    pub fn test_arm_card_success_hook(&self) {
        self.scope().card_success_hook.arm();
    }

    /// 等钩子报到，返回插件 ID。
    #[doc(hidden)]
    pub async fn test_wait_card_success_arrived(&self) -> String {
        let mut arrivals = self.scope().card_success_hook.arrivals();
        loop {
            if let Some(plugin_id) = arrivals.borrow().clone() {
                return plugin_id;
            }
            if arrivals.changed().await.is_err() {
                panic!("the card success test hook is gone");
            }
        }
    }

    /// 放行卡住的决定。
    #[doc(hidden)]
    pub fn test_release_card_success(&self) {
        self.scope().card_success_hook.release();
    }

    /// 每次发布引导视图前先回调（发布用的那把锁持着，回调里看到的是即将发出去的那份视图和已记下发过消息的房间）。
    #[doc(hidden)]
    pub fn test_set_onboarding_publish_hook(
        &self,
        hook: Option<crate::onboarding::TestPublishHook>,
    ) {
        *self
            .scope()
            .onboarding
            .test_publish_hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = hook;
    }

    /// 像 App 被杀那样停下：关掉当前作用域，等它运行时里的任务都结束再返回。
    ///
    /// 释放核心只在后台关运行时、不等（壳可能在主线程上释放），旧作用域的任务这时还可能活着。同一进程里紧接着在同一个
    /// 数据目录上开新核心，matrix-sdk 存储锁就换持有者名（见 `wonton_matrix` 的 `store_holder`），新 Client 头一次
    /// 拿到锁时换掉 Olm 机器，重启后马上发的加密消息会卡到发送期限。真的重启是新进程，不会这样；测试模拟重启前先调这个。
    #[doc(hidden)]
    pub async fn test_shut_down(&self) {
        let scope = self.scope();
        tokio::task::spawn_blocking(move || scope.close(true))
            .await
            .expect("closing the scope panicked");
    }

    /// 推进连接质量判定的时钟并采样真实 QUIC 计数；窗口和滞回门槛保持生产值。
    #[doc(hidden)]
    pub async fn test_sample_connection_quality(
        &self,
        elapsed: std::time::Duration,
    ) -> wonton_protocol::HomeViewModel {
        self.scope().home.test_sample_quality(elapsed).await
    }
}

impl ClientCore {
    /// 本设备身份的 endpoint 绑定之前、在跑绑定的阻塞线程里调用的钩子：一直不返回就是卡住的 iroh 绑定。
    #[doc(hidden)]
    pub fn test_set_bind_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self
            .scope()
            .home
            .test_hooks
            .bind
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = hook;
    }

    /// 替掉配对的每一段时限（绑定、一次交换、一次确认）和看门线程的时限，免得测试等满生产的三段 30 秒。
    #[doc(hidden)]
    pub fn test_set_pair_deadlines(&self, stage: Duration, watchdog: Duration) {
        *self
            .scope()
            .home
            .test_hooks
            .pair_deadlines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((stage, watchdog));
    }
}
