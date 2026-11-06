//! 确认卡片成功后补取已装列表（ADR-0016）：卡片装好后壳凭 [`InstalledPlugin`] 显示"打开/放到底栏"，
//! 主人批准时正盯着卡片，列表却常常还是旧的，要进一次"应用"才有。聊天那头只报本会话里见过的卡片变成
//! Succeeded 的（亲眼见过它等回答，或这台设备回答过它），启动时翻回来的历史结果不报——那时
//! [`follow_home`](super::follow_home) 自己会取。

use std::sync::Arc;

use super::PluginsLink;

impl PluginsLink {
    /// 亲眼看到一张带插件的卡片变成"执行完了"。插件已在列表里就不发请求；隧道断着或退到后台时
    /// 也不发——请求会把重连变成主动行为，连上后 [`follow_home`](super::follow_home) 自己会取。
    /// 插件确实已不存在时只取这一次，不重试。
    pub(crate) fn card_succeeded(self: &Arc<Self>, plugin_id: &str) {
        let known = self
            .view
            .borrow()
            .installed
            .as_ref()
            .is_some_and(|installed| {
                installed
                    .plugins
                    .iter()
                    .any(|plugin| plugin.id == plugin_id)
            });
        if known || !self.can_poll() {
            return;
        }
        self.refresh_installed();
    }
}
