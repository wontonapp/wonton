//! 只给测试用的时序钩子（cargo feature `test-hooks`，生产构建不编进来）：把“补不补取已装列表”的决定
//! 卡住，测试好验证卡片进了 settled_confirmations 时决定已经做完。集成测试经 `ClientCore` 的
//! `test_*` 方法打开、等报到、放行。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

use crate::plugins::PluginsLink;

/// 决定的测试钩子，挂在创建它的那个核心上。没打开时 [`arrive`](Self::arrive) 直通，和生产路径一样。
pub(crate) struct CardSuccessHook {
    armed: AtomicBool,
    /// 最近一次报到的插件 ID，`None` 是还没报到。
    arrived: watch::Sender<Option<String>>,
    /// 放行没有。放行是一次性的：一次验证里决定只卡一张卡片，之后的报到不再等。
    released: watch::Sender<bool>,
}

impl CardSuccessHook {
    pub(crate) fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            arrived: watch::Sender::new(None),
            released: watch::Sender::new(false),
        }
    }

    /// 打开：之后的报到先报、等一次放行。
    pub(crate) fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
        let _ = self.arrived.send(None);
        let _ = self.released.send(false);
    }

    /// 钩子开着就先报“到了”（带插件 ID），等放行后才回来；不开就直通。
    pub(crate) async fn arrive(&self, plugin_id: &str) {
        if !self.armed.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.arrived.send(Some(plugin_id.to_owned()));
        let mut released = self.released.subscribe();
        while !*released.borrow_and_update() {
            if released.changed().await.is_err() {
                return;
            }
        }
    }

    pub(crate) fn release(&self) {
        let _ = self.released.send(true);
    }

    /// 报到的流：测试等它。
    pub(crate) fn arrivals(&self) -> watch::Receiver<Option<String>> {
        self.arrived.subscribe()
    }
}

/// 把决定包进钩子：报到、等放行，然后才补不补取。等的方式是协作的（future），不占住运行时的线程。
pub(crate) fn wrap(
    plugins: Arc<PluginsLink>,
    hook: Arc<CardSuccessHook>,
) -> crate::chat::CardSuccess {
    Arc::new(move |plugin_id: &str| {
        let plugins = plugins.clone();
        let hook = hook.clone();
        Box::pin(async move {
            hook.arrive(plugin_id).await;
            plugins.card_succeeded(plugin_id)
        })
    })
}
