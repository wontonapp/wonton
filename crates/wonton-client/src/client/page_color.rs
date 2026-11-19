use super::ClientCore;
use crate::pages::color;

#[uniffi::export]
impl ClientCore {
    /// 壳在整页网页主框架的文档开始前注入的取色脚本：页面顶上实际显示的颜色变了，就经
    /// `window.wontonPageColor.postMessage` 报一个 `#rrggbb`，没有可用的颜色时报空串。
    pub fn page_color_script(&self) -> String {
        color::script()
    }
}
