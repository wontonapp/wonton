//! 页面顶上实际显示的颜色：两端的壳注入同一份脚本，经 `wontonPageColor` 通道收 `#rrggbb`（读法见 `script.js`）。

pub(crate) fn script() -> String {
    include_str!("script.js").to_owned()
}
