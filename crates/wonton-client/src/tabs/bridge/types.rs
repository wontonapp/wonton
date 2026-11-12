use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TabPlatform {
    Ios,
    Android,
}

/// WebView 坐标中的安全区，单位为 CSS px。
#[derive(Clone, Copy, Debug, Serialize, uniffi::Record)]
pub struct TabSafeArea {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct TabEnvironment {
    pub locale: String,
    pub time_zone: String,
    pub platform: TabPlatform,
    /// 壳按系统和 App 设置合成后的明暗。
    pub dark: bool,
    /// 含安全区的栏高；全屏时是胶囊下沿和 Home 条。Inset 布局由核心归零。
    pub inset_top: f64,
    pub inset_bottom: f64,
    pub safe_area: TabSafeArea,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TabBridgeAction {
    OpenLink { url: String },
    DraftMessage { text: String },
}

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct TabBridgeResult {
    pub reply_script: Option<String>,
    pub action: Option<TabBridgeAction>,
    /// 收到 initialized 通知后壳可以撤掉骨架。
    pub initialized: bool,
}
