//! 各插件 Bot 的同步时间：据它判断长轮询是不是已经停了。

use super::super::manifest::PluginId;
use super::PluginManager;

/// Bot 的长轮询最长挂 30 秒：超过这么久没有一次成功的同步，就是停了，不是正好在等消息。
const STALLED_SYNC_SECS: u64 = 120;

impl PluginManager {
    /// 这个插件的 Bot 刚同步成功过一次。
    ///
    /// 记的是时间不是“健康”：Bot 走了长轮询，进程活着、健康检查说 connected 都可能只是状态文件的残影，
    /// 只有还经核心同步才算真的收得到消息（见 [`Self::stalled_bot_sync`]）。
    pub(crate) fn note_bot_sync(&self, id: &PluginId) {
        self.bot_syncs
            .write()
            .expect("Bot 同步时间的锁中毒")
            .insert(id.clone(), tokio::time::Instant::now());
    }

    /// 这个插件的 Bot 多久没同步了（秒），超过 [`STALLED_SYNC_SECS`] 才算停了。从没同步过的给 `None`：
    /// 不用长轮询的 Bot 插件不该被这条检查报异常。
    pub(crate) fn stalled_bot_sync(&self, id: &PluginId) -> Option<u64> {
        let syncs = self.bot_syncs.read().expect("Bot 同步时间的锁中毒");
        let age = syncs.get(id)?.elapsed().as_secs();
        (age > STALLED_SYNC_SECS).then_some(age)
    }

    /// 这个插件换了一份新实例（安装、启动、恢复重建、卸载）：上一次实例的同步时间不再说明问题。
    ///
    /// 不重置的话，停用超过两分钟的插件一启动就会被 [`Self::check_bot_sync`] 判成异常，Bot 还没来得及同步。
    pub(crate) fn forget_bot_sync(&self, id: &PluginId) {
        self.bot_syncs
            .write()
            .expect("Bot 同步时间的锁中毒")
            .remove(id);
    }
}
