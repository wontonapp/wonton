//! 主界面的底栏和收件箱铃铛（ADR-0126、ADR-0095）。
//!
//! 底栏只画在两个根页上，成员和顺序存在账号数据 [`crate::LAYOUT_EVENT`] 的
//! [`LayoutSettings::bar`](crate::LayoutSettings::bar)；壳只渲染 [`BarViewModel`]、发
//! [`PageAction`](crate::PageAction) 的四个底栏操作。收件箱不在底栏上：铃铛的数字来自收件箱接口的
//! [`InboxCounts::needing`](crate::InboxCounts::needing)，不由壳自己数。

use serde::{Deserialize, Serialize};

use crate::{InboxViewModel, MyAppRef, PluginIcon};

/// 底栏最多几项。满了以后 [`PageAction::AddBarItem`](crate::PageAction::AddBarItem) 返回 `InvalidInput`；
/// [`BarViewModel::max_items`] 和它同一个类型。
pub const MAX_BAR_ITEMS: u32 = 6;

/// 底栏的一项：两个内置根页，或用户从“我的应用”加上来的一项。
///
/// 用户可以把 `Agent`、`Apps` 也删掉（ADR-0126 的“没选的方案”）；删了从“我的应用”页加回来。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BarEntry {
    /// 默认 Agent 的会话列表。
    Agent,
    /// 应用页。
    Apps,
    /// 我的应用里的一行。点它和在“我的应用”里点它一样打开整页，不带底栏。
    /// `MyAppRef` 自己也用 `type` 作标签，所以包一层 `app`，不做成元组变体。
    App { app: MyAppRef },
}

/// 账号数据里没有 `bar` 时的底栏：Agent、应用。显式存了空表时仍按最短的一项算，见 [`check_bar`]。
pub fn default_bar() -> Vec<BarEntry> {
    vec![BarEntry::Agent, BarEntry::Apps]
}

/// 底栏的一组成员为什么不行。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BarProblem {
    /// 超过 [`MAX_BAR_ITEMS`]。
    Full,
    /// 同一项出现了两次。
    Duplicate,
    /// 一项都没有：两个根页删光也要留至少一项，不然主界面没有开口。
    Empty,
    #[serde(other)]
    Other,
}

/// 底栏成员的规则：核心加项和收下别的设备存的栏时都用它查。空表也不行（[`BarProblem::Empty`]），
/// 没选过时的空栏先落 [`default_bar`]。
pub fn check_bar(entries: &[BarEntry]) -> Result<(), BarProblem> {
    if entries.is_empty() {
        return Err(BarProblem::Empty);
    }
    if entries.len() > MAX_BAR_ITEMS as usize {
        return Err(BarProblem::Full);
    }
    for (index, entry) in entries.iter().enumerate() {
        if entries[..index].contains(entry) {
            return Err(BarProblem::Duplicate);
        }
    }
    Ok(())
}

/// 底栏一项的图标。内置两项是固定图标，“Agent”那一项的选中图标是云朵（ADR-0126）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BarIcon {
    Agent,
    Apps,
    /// 插件自己的彩色图标；公网网页和没有图标的插件为空，壳画占位。
    ///
    /// 包一层 `icon`（不学 [`TabRef::Plugin`](crate::TabRef::Plugin) 那样带独立字段）是因为同一个变体要能表示
    /// “没有图标”，和上面两个固定图标的变体区分开。
    App {
        icon: Option<PluginIcon>,
    },
}

/// 底栏的一项和它的展示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BarItem {
    pub entry: BarEntry,
    /// 读屏标签：底栏图标下面不写字。
    pub label: String,
    pub icon: BarIcon,
    /// `Agent`：主 Agent 会话的未读；`Apps`：出错的插件加可升级的插件；应用项为 0。0 不画。
    pub badge: u32,
}

/// 底栏。只在两个根页上画；压着页面时不画。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BarViewModel {
    /// 成员和顺序照 [`LayoutSettings::bar`](crate::LayoutSettings::bar)；插件卸掉的那一项不在这里。
    pub items: Vec<BarItem>,
    /// [`MAX_BAR_ITEMS`]。
    pub max_items: u32,
    /// 刚才那次加项因为满了被拒；下一次加成功时清掉。开着时壳把“加到 底栏”显示成满。
    pub full: bool,
}

/// 收件箱铃铛：数字是等人回答的条数，点开进收件箱页。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxBadge {
    /// 收件箱接口的 `counts.needing`，为 0 时铃铛旁不画数字。为空是不确定（还没取到、或最近一次没取成）：
    /// 壳画成不确定，不画成 0（ADR-0126 的“代价与风险”）。
    pub needing: Option<u32>,
}

impl InboxBadge {
    /// 从收件箱的 ViewModel 取：铃铛和收件箱页用同一个响应，不另开请求。
    pub fn of(inbox: &InboxViewModel) -> Self {
        let known = !inbox.loading && inbox.problem.is_none();
        Self {
            needing: known.then_some(inbox.list.counts.needing),
        }
    }
}

#[cfg(test)]
#[path = "bar_tests.rs"]
mod tests;
