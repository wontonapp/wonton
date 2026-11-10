//! 页面实例的分类：哪些是插件页、哪些是网页、哪些能从栏上拿掉。

use wonton_protocol::{TabLayout, TabRef};

pub(crate) fn normalize_layout(layout: TabLayout) -> TabLayout {
    if layout == TabLayout::Other {
        TabLayout::Inset
    } else {
        layout
    }
}

pub(crate) fn is_plugin(tab: &TabRef) -> bool {
    matches!(tab, TabRef::Plugin { .. } | TabRef::SharedApp { .. })
}

/// 绑在插件上的页面：插件页和插件里的某一页。换服务器时它们跟着走，公网网页不绑服务器。
pub(crate) fn is_plugin_page(tab: &TabRef) -> bool {
    matches!(
        tab,
        TabRef::Plugin { .. } | TabRef::PluginPath { .. } | TabRef::SharedApp { .. }
    )
}

/// 是网页的三种页面：插件页、插件里的某一页、公网网页。加载失败和重新加载都按网页处理。
pub(crate) fn is_web_page(tab: &TabRef) -> bool {
    matches!(
        tab,
        TabRef::Plugin { .. }
            | TabRef::PluginPath { .. }
            | TabRef::Url { .. }
            | TabRef::SharedApp { .. }
    )
}

pub(crate) fn is_removable(tab: &TabRef) -> bool {
    // “应用”只在没配对时在栏上，那时它是配对页，也不能拿。
    !matches!(tab, TabRef::Chat | TabRef::Apps)
}
