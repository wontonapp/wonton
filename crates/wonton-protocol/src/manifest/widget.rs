use serde::{Deserialize, Serialize};

/// 房间顶栏的图标，清单和 App 用同一个枚举：不认识的落到 `Other`。
pub use crate::WidgetEntry;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixWidget {
    /// 网页入口下的路径和查询，如 `/room?widgetId=$matrix_widget_id`。占位符由 widget 规范和核心替换，
    /// 路径规则同 `webui.path`。
    pub url: String,
    #[serde(default)]
    pub entry: WidgetEntry,
    /// 页面要的能力，如 `org.matrix.msc2762.send.state_event:org.matrix.msc3401.call.member`。
    #[serde(default)]
    pub capabilities: Vec<String>,
}
