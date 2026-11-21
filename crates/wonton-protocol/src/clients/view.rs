//! 公网访问的两个页面（ADR-0021、ADR-0089、ADR-0090）：设置页和详情页。壳只渲染，不再自己推导。
//!
//! 核心可能比 App 新：多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

use serde::{Deserialize, Serialize};

use super::{
    ClientsPlugin, ClientsProblem, DomainSettings, EntryBlock, EntryKind, GuestLink,
    IngressSettings, ProbeResult, PublicAccess, PublicItem, PublicReach, PublicSwitchState,
};

/// 公网详情页：谁公开、三道闸门、自检和访客链接。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsViewModel {
    /// 页面加载到哪一步。
    pub state: ClientsState,
    /// 有入口的应用，按显示名排；每个应用的入口先是局域网、再公网，最后按名字排 `clients.tcp`。
    pub plugins: Vec<ClientsPlugin>,
    /// 域名和证书；还没取到时为空。
    pub domain: Option<DomainSettings>,
    /// 入口的种类和设置；还没选入口时为空。
    pub ingress: Option<IngressSettings>,
    pub public_access: PublicAccess,
    /// 总开关关过、或者在关的时候：三道闸门各自怎么样；没关过时为空。
    pub gates: Option<PublicSwitchState>,
    /// 最近一次自检；还没跑过时是 [`ProbeResult::NotRun`]。
    pub self_probe: ProbeResult,
    /// 现在公开的应用、路径和访客链接。
    pub public_now: Vec<PublicItem>,
    pub public_reach: PublicReach,
    /// 按应用收集的访客链接；没打开过那个应用的链接列表时没有这一项。
    pub guests: Vec<ClientsGuests>,
    /// 现在开不了的入口和原因，页顶的汇总用它；列表行里也各有一份。
    pub blocked: Vec<ClientsBlocked>,
    /// 正在保存；期间照样显示上次取到的内容。
    pub saving: bool,
    /// 最近一次操作被拒的类别；下一次成功时清空。
    pub problem: Option<ClientsProblem>,
}

/// 设置页上的公网访问：总开关、入口和域名，以及关掉时会失效的东西。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsSettingsViewModel {
    pub state: ClientsState,
    pub public_access: PublicAccess,
    pub public_reach: PublicReach,
    /// 总开关关过、或者在关的时候：三道闸门各自怎么样，“没关干净”按它显示。
    pub gates: Option<PublicSwitchState>,
    pub ingress: Option<IngressSettings>,
    pub domain: Option<DomainSettings>,
    /// 入口和域名都配好了，开关能打开；缺一样时壳把开关置灰并链到那一页。
    pub can_enable: bool,
    /// 现在开着公网入口的应用数。
    pub open_apps: u32,
    /// 关掉总开关会一起失效的应用、路径和访客链接；开关已经关着时是空的。
    pub closing: Vec<PublicItem>,
    pub saving: bool,
    pub problem: Option<ClientsProblem>,
}

/// 一个应用的一组访客链接；取或改这一项失败时 `problem` 说明原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsGuests {
    pub plugin_id: String,
    /// 按新建时间排。
    pub links: Vec<GuestLink>,
    pub problem: Option<ClientsProblem>,
}

/// 现在开不了的一个入口和原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsBlocked {
    pub plugin_id: String,
    pub kind: EntryKind,
    pub reason: EntryBlock,
}

/// 页面取到哪一步。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClientsState {
    /// 正在取；期间显示上次取到的内容。
    Loading,
    Ready,
    Failed {
        problem: ClientsProblem,
    },
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}
