//! UI 测试用的假数据入口：把 App 直接开进主界面，不需要家里的服务器。
//!
//! 整个模块在 cargo feature `ui-fake` 后面，生产 Release 包不带（构建脚本按 profile 开）；Swift 侧的调用
//! 另在 `#if DEBUG` 里。开着时 `new_ui_fake` 仍然建一个正常的 `ClientCore`（没有后端），再把塞进订阅的
//! ViewModel 换成写死的值；开着时 `dispatch` 不落到作用域，免得网络链路把 ViewModel 改回空的。
//!
//! 只够渲染首屏和主入口的导航：一台“家里”的服务器、一个 Bot 私聊、一个有人在通话、顶栏带 widget 入口的群聊
//! （ADR-0096）、一个没有网页的已装应用（点开是详情页）、一个带授权设备的应用（`ui_fake_plugin_detail.rs`）、
//! 三个有网页的已装应用（页面各停在加载中、已停止、出了问题）、一个整页打开本机网页的应用、
//! 主面板里钉着的一页、一台点开是原生音乐界面的假曲库（`music.rs`）、一个点开是原生相册页的假相册应用（`photos.rs`）、商店入口、“等你确认”里一张带设备的安装卡片（`ui_fake_devices.rs`）、引导已经走完。
//! 操作里只认开页、关页（`OpenPage`、`ClosePage`）、回答确认卡片、重试失败的安装（`ui_fake_failed_install.rs`），
//! 以及时间线上界面测试约定的那几个（`ui_fake_timeline.rs`）：停下在跑的轮次（`ui_fake_agent_stop.rs`，停成了连在等
//! 的卡片一起撤回），开关页照真核心的规则改页面栈和应用网格的开关，壳的出口才有东西可测；
//! 没有隧道，插件的网页本身加载不出来；房间的时间线和加载得出来的本机网页（面板里一张、整页一张）在 `ui_fake_room/`。
//! 商店列表和别的操作后的新状态不在里面，要用这些的测试仍然连开发栈（见
//! `apps/ios/WontonUITests/UITestBackend.swift`）。假房间里的照片例外：`ui_fake_room` 带同组多张的
//! 样张，字节由 `load_media` 在假数据入口下直接给，图像类走查不用连栈。
//!
//! 本文件只留子模块声明和给 `client` 其他模块的再导出；夹具按职责分在 `ui_fake/` 下：网页标签与 widget
//! （`tabs.rs`）、房间与聊天（`chat.rs`）、应用网格与分享（`plugins.rs`）、主面板（`pages.rs`）、首屏与配对码
//! （`home.rs`）、引导（`onboarding.rs`）、启动（`entry.rs`）、操作分发（`dispatch.rs`）、测试（`tests.rs`）。

mod chat;
mod dispatch;
mod entry;
mod home;
mod music;
mod onboarding;
mod pages;
mod photos;
mod plugins;
mod tabs;

/// 假房间和假插件给 `client` 其他模块的入口：再导出保持原来的 `super::ui_fake::…` 路径不变。
pub(super) use chat::{CALL_ROOM_ID, ROOM_ID, mention_candidates};
pub(super) use music::music_link;
pub(super) use plugins::fake_plugin;
pub(super) use tabs::call_widget;

#[cfg(test)]
mod tests;
