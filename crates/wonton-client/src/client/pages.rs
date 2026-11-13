//! 页面的壳接口：注入脚本、桥消息和网页权限的询问，拆出 [client mod] 保持文件在门禁内。

use wonton_protocol::{CoreError, PlacedTab, TabRef};

use super::ClientCore;
use super::sharing;
use crate::pages::bridge::{self, TabBridgeResult, TabEnvironment};
use crate::pages::media_permission::TabMediaPermission;

#[uniffi::export]
impl ClientCore {
    /// 文档开始前、只在插件自己的主框架注入；原生通道名为 `wontonBridge`。
    pub fn tab_document_script(
        &self,
        page: PlacedTab,
        environment: TabEnvironment,
    ) -> Result<String, CoreError> {
        bridge::document_script(
            &self.scope().pages,
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
            &self.scope().pages,
            &self.appearance,
            &page,
            is_main_frame,
            &origin,
            &message,
            &environment,
        )
    }

    /// widget 页（[`TabRef::Widget`]，ADR-0096）的地址出来、开始加载后订阅：每一项是驱动发给页面的消息包成的脚本，
    /// 壳原样注入同一个页面的主框架。页面发来的 widget 报文照常经 [`Self::tab_handle_message`] 交进来。
    /// 不是 widget 页时是输入非法，驱动还没起来（页面没打开、地址还没出来）是未就绪。
    pub fn subscribe_tab_widget(
        &self,
        page: PlacedTab,
    ) -> Result<std::sync::Arc<bridge::TabWidgetSubscription>, CoreError> {
        bridge::subscribe(&self.scope().pages, &page)
    }

    /// 外观、字号、布局或全屏状态变化后注入，不重新加载页面。
    pub fn tab_context_script(
        &self,
        page: PlacedTab,
        environment: TabEnvironment,
    ) -> Result<String, CoreError> {
        bridge::context_script(
            &self.scope().pages,
            &self.appearance.view(),
            &page,
            &environment,
        )
    }

    pub fn tab_reselected_script(&self) -> String {
        bridge::reselected_script()
    }

    /// 连接变化时派发真实的核心状态；不把不确定状态当成已连接。
    pub fn tab_connection_script(&self, tab: TabRef) -> String {
        let scope = self.scope();
        let home = scope.home.sender();
        let plugins = scope.plugins.sender();
        bridge::connection_script(sharing::tab_connected(
            &tab,
            &home.borrow().status,
            &plugins.borrow(),
        ))
    }

    /// 插件页在问摄像头/麦克风：核心按插件记着的决定放行、拒绝，没问过的出询问
    /// （[`PagesViewModel::media_prompt`](wonton_protocol::PagesViewModel)）。`origin` 是发请求的网页源，壳从 WebView 的原生回调取；
    /// 页面里嵌的其他源（如第三方 iframe）一律拒。
    pub fn tab_media_permission_request(
        &self,
        page: PlacedTab,
        origin: String,
        camera: bool,
        microphone: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        self.scope()
            .pages
            .request_media_permission(&page, &origin, camera, microphone)
    }

    /// 壳交回询问的回答：核心记下决定，返回该给网页的结果（允许 / 拒绝）。
    pub fn tab_media_permission_decide(
        &self,
        allow: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        self.scope().pages.decide_media_permission(allow)
    }

    /// 插件详情页的开关：`true` 是允许，`false` 是改回没问过（清掉，下次再问）。
    /// 拒绝只在询问弹窗里产生，开关给不出拒绝。不经 [`Action`](wonton_protocol::Action)：
    /// 那个枚举的 uniffi 元数据已贴着上限，加变体会编不过。
    pub fn set_plugin_media_permission(
        &self,
        plugin_id: String,
        allow: bool,
    ) -> Result<(), CoreError> {
        self.scope().plugins.set_media_permission(plugin_id, allow)
    }
}
