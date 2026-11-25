//! “关于”里的隐私说明和开源许可：内容都由核心给，壳只负责排。
//!
//! 许可清单是构建时用现成工具生成的 `licenses.json`（cargo-about 取 Rust crate、AboutLibraries 取 Android 依赖），
//! 壳把 JSON 交给核心解析；许可全文一并带在清单里，打开某个许可时不用再问一次。

use serde::{Deserialize, Serialize};

/// 隐私说明。正文按段落拆好，壳不做 Markdown。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrivacyViewModel {
    pub sections: Vec<PrivacySection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrivacySection {
    pub title: String,
    pub paragraphs: Vec<String>,
}

/// 开源许可清单。一个许可一组，组里是用到它的组件，`text` 是许可全文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LicensesViewModel {
    pub licenses: Vec<LicenseGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LicenseGroup {
    /// 许可 id（SPDX id，如 `MIT`），壳里当列表的稳定标识；两个许可可能同名，不能拿名字当键。
    pub id: String,
    /// 列表上的分组标题，取来源里的许可名；Rust 侧就是 SPDX id，Android 侧是 POM 里的名，可能和 id 相同。
    pub name: String,
    pub components: Vec<LicenseComponent>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LicenseComponent {
    /// crate 名或 `group:artifact`。
    pub name: String,
    pub version: String,
}
