//! 底栏页面的壳接口：注入脚本、桥消息和网页权限的询问，拆出 [client mod] 保持文件在门禁内。

use wonton_protocol::{CoreError, PlacedTab};

use super::ClientCore;
use crate::tabs::bridge::{self, TabBridgeResult, TabEnvironment};
use crate::tabs::media_permission::TabMediaPermission;

#[uniffi::export]
impl ClientCore {
    /// 文档开始前、只在插件自己的主框架注入；原生通道名为 `wontonBridge`。
    pub fn tab_document_script(
        &self,
        page: PlacedTab,
        environment: TabEnvironment,
    ) -> Result<String, CoreError> {
        bridge::document_script(
            &self.scope().tabs,
            &self.appearance.view(),
            &page,
            &environment,
        )
    }

    /// 壳原样交入消息及原生回调的主框架标志、来源；核心校验来源，草稿不发送。
    pub fn tab_handle_message(
        &self,
        page: PlacedTab,
        is_main_frame: bool,
        origin: String,
        message: String,
        environment: TabEnvironment,
    ) -> TabBridgeResult {
        bridge::handle_message(
            &self.scope().tabs,
            &self.appearance,
            &page,
            is_main_frame,
            &origin,
            &message,
            &environment,
        )
    }

    /// 外观、字号、布局或全屏状态变化后注入，不重新加载页面。
    pub fn tab_context_script(
        &self,
        page: PlacedTab,
        environment: TabEnvironment,
    ) -> Result<String, CoreError> {
        bridge::context_script(
            &self.scope().tabs,
            &self.appearance.view(),
            &page,
            &environment,
        )
    }

    pub fn tab_reselected_script(&self) -> String {
        bridge::reselected_script()
    }

    /// 连接变化时派发真实的核心状态；不把不确定状态当成已连接。
    pub fn tab_connection_script(&self) -> String {
        bridge::connection_script(&self.scope().home.view().status)
    }

    /// 插件页在问摄像头/麦克风：核心按插件记着的决定放行、拒绝，没问过的出询问
    /// （[`TabsViewModel::media_prompt`]）。`origin` 是发请求的网页源，壳从 WebView 的原生回调取；
    /// 页面里嵌的其他源（如第三方 iframe）一律拒。
    pub fn tab_media_permission_request(
        &self,
        page: PlacedTab,
        origin: String,
        camera: bool,
        microphone: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        self.scope()
            .tabs
            .request_media_permission(&page, &origin, camera, microphone)
    }

    /// 壳交回询问的回答：核心记下决定，返回该给网页的结果（允许 / 拒绝）。
    pub fn tab_media_permission_decide(
        &self,
        allow: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        self.scope().tabs.decide_media_permission(allow)
    }
}
