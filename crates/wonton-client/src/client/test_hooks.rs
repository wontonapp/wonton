//! 只给测试用的钩子接口（feature `test-hooks`，生产构建没有）：打开、等报到、放行。
//! 钩子挂在当前作用域上（`Scope::open` 建的），换服务器会换作用域，测试不换服务器就一直是同一个。
//! 状态和等放行的机制在 [`crate::test_hooks`]。

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
}
