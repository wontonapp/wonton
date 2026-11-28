use serde::{Deserialize, Serialize};

use crate::TabRef;

/// 页面栈、我的应用和面板操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PageAction {
    /// 打开一个页面：压进栈（[`PagesViewModel::stack`](crate::PagesViewModel::stack)），已经在栈里就挪到栈顶。
    /// 打开根页面（聊天、应用网格页）等于一路弹回根。插件页要等插件列表读到。
    OpenPage { tab: TabRef },
    /// 关掉当前页，露出下面一层。
    ClosePage,
    /// 加进我的应用的 `index` 位置，不给就加到末尾。
    AddMyApp {
        app: crate::MyAppRef,
        title: Option<String>,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        index: Option<u32>,
    },
    /// 从我的应用拿掉。
    RemoveMyApp { app: crate::MyAppRef },
    /// 把一行移到 `index`（移动后的下标，从 0 开始）。
    MoveMyApp { app: crate::MyAppRef, index: u32 },
    /// 新装的应用自动加到我的应用末尾。
    SetAutoAddApps { enabled: bool },
    /// 重新和账号数据对一次账（侧边栏“我的应用”过期提示的重试）。没有会话时无事可做，等重连后自动对账。
    RetryMyAppSync,
    /// 按网址把一页加进我的应用：插件网站下的地址作为插件页面，其他网址必须是 `https://`，
    /// Bot 的网页不收。`title` 为空时核心按插件或域名补全。
    AddMyAppUrl { url: String, title: Option<String> },
    /// 把插件页面或 `https://` 公网页面放进面板末尾；重复、已满或不允许放入时返回 `InvalidInput`。
    /// 写入账号数据成功才生效，连接失败时不改变面板。
    AddPanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
    },
    /// 从面板拿掉。不在面板里时返回 `InvalidInput`。
    RemovePanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
    },
    /// 把面板里的一项移到 `index`（移动后的下标，从 0 开始）。不在面板里或下标越界时返回 `InvalidInput`。
    MovePanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
        index: u32,
    },
    /// 壳报告正在看面板的哪一页：核心记住它（本机，下次拉开还是它），并清掉这一页的红点。
    /// 拉开面板和翻页时都发。
    SelectPanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
    },
    /// 收回或给回一个 Bot 往主面板钉页的同意（设置里）；只改以后还问不问，已经钉上的页面不动。
    SetBotPinConsent {
        plugin_id: String,
        bot: String,
        allowed: bool,
    },
    /// 按当前 `url` 把页面钉到面板或我的应用：插件入口下的路径作为插件页面，其他网址必须是 `https://`。
    /// `title` 为空时核心按插件或域名补全。
    PinCurrentPage {
        url: String,
        title: Option<String>,
        to: crate::PinTarget,
    },
    /// 重新加载插件页（重试、“···”里的重新加载）。`page` 指是哪一份：栈里的和面板里的同一页面
    /// 各自加载，互不影响（ADR-0052）。
    ReloadTab { page: crate::PlacedTab },
    /// 壳报告网页加载失败。`status` 是 HTTP 状态码，没拿到响应时为空；原因由核心判断。
    /// `page` 指是哪一份实例。
    TabLoadFailed {
        page: crate::PlacedTab,
        status: Option<u16>,
    },
    /// 壳报告网页加载成功（如按返回回到成功的历史页），核心清掉这个页面的加载失败显示。
    TabLoadSucceeded { page: crate::PlacedTab },
    /// 点底栏的一项（ADR-0126）：两个根页之间切换；应用项打开它的整页。
    SelectBarItem { entry: crate::BarEntry },
    /// 主人看过了“已卸载，已从我的应用移除”的提示。
    DismissUninstalledTabs,
    /// 往底栏末尾放一项：根页可以删出栏再加回来，应用项要在“我的应用”里。
    ///
    /// 已在栏上、`App` 的行不在我的应用里、或满了 [`MAX_BAR_ITEMS`](crate::MAX_BAR_ITEMS) 时返回 `InvalidInput`
    /// 并说明原因（[`BarProblem`](crate::BarProblem)）；写成功后清掉 [`BarViewModel::full`](crate::BarViewModel::full)。
    AddBarItem { entry: crate::BarEntry },
    /// 从底栏拿掉一项，内置的两项也能拿掉；栏上只剩一项时返回 `InvalidInput`（[`BarProblem::Empty`](crate::BarProblem::Empty)）。
    /// 不在栏上时也返回 `InvalidInput`。
    RemoveBarItem { entry: crate::BarEntry },
    /// 把底栏第 `from` 项移到 `to`（移动后的下标，从 0 开始）。越界时返回 `InvalidInput`。
    MoveBarItem { from: u32, to: u32 },
}
