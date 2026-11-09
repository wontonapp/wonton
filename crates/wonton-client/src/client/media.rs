use super::ClientCore;
use crate::tabs::media;
use crate::tabs::media::{PageMediaAction, PageMediaState};

#[uniffi::export]
impl ClientCore {
    /// Android 在插件主框架的文档开始前注入的标准媒体适配脚本。
    pub fn page_media_script(&self) -> String {
        media::script()
    }

    /// 解析网页媒体快照；无效消息和不支持的版本不改变系统状态。
    pub fn page_media_message(&self, message: String) -> Option<PageMediaState> {
        media::message(&message)
    }

    /// 网页媒体开始或停止播放：播放期间退到后台也不断开隧道。
    pub fn page_media_active(&self, active: bool) {
        self.scope().home.page_media_active(active);
    }

    /// 系统媒体操作转成注入原页面的脚本，跳转与快进快退的参数单位为毫秒。
    pub fn page_media_command_script(
        &self,
        action: PageMediaAction,
        seek_ms: Option<u64>,
    ) -> String {
        media::command_script(action, seek_ms)
    }
}
