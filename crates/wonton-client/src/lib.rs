//! 客户端厚核心：状态、业务逻辑、网络与 ViewModel，经 UniFFI 导出给 SwiftUI 与 Compose。
//!
//! 接口形态是“操作进、ViewModel 出”，并保持可序列化，以便测试和将来的 Web 端复用。
//! 设计见 `docs/adr/0005-native-clients-thick-core.md`。
//!
//! - 操作进：壳调用 [`ClientCore::dispatch`]，参数是 [`Action`]。
//! - ViewModel 出：壳调用 [`ClientCore::subscribe_session_list`]、[`ClientCore::subscribe_home`]、
//!   [`ClientCore::subscribe_chat`]、[`ClientCore::subscribe_plugins`]、[`ClientCore::subscribe_onboarding`]、[`ClientCore::subscribe_transfers`]、[`ClientCore::subscribe_appearance`]，循环等待订阅的 `next()`，包装成 Swift 的 `AsyncSequence` 或 Kotlin 的 `Flow`。打开的房间的时间线用 [`ClientCore::subscribe_timeline`]，推的是不能合并的
//!   增量批次。
//!
//! 和家里服务器的连接见 `home` 模块：iroh 隧道加 WebView 和 HTTP 客户端用的本地代理（ADR-0004）。聊天见
//! `chat` 模块：经隧道登录主人的 Matrix 账号（ADR-0003、ADR-0009）。推送见 `push` 模块：登记推送订阅，收到推送
//! 时取回消息在本机解密（ADR-0003 的“设计：加密推送”）。文件见 `files` 模块：传输列表、卡片和给播放器的地址（ADR-0020）。会话列表还是 M0 验证 UniFFI 用的示例，
//! 只放在内存里。
//!
//! 引导卡片由 [`OnboardingViewModel::nav`] 控制按钮；账号登录仍直接发 [`Action::ContinueOnboarding`]。
//! 上一步不撤销已保存的资料、默认 Agent、key 或安装，也保留本机选项；已配置模型时，反向导航跳回 Agent，
//! 不把主人送回 Apps。插件安装完成会静默更新推荐行，保存 key 和已有错误不受影响（ADR-0034、ADR-0038）。
//! 后台通知不倒退已发布的引导状态；新操作接替旧操作的忙碌标记，失败后仍能返回或跳过。

// matrix-sdk 的 future 嵌套很深，默认的 128 层不够算出聊天后台任务的类型布局。
#![recursion_limit = "256"]

uniffi::setup_scaffolding!();

mod agent_chains;
#[cfg(target_os = "android")]
mod android;
mod appearance;
mod bot_contacts;
mod browser;
mod chat;
mod client;
mod composer;
mod files;
mod friends;
mod home;
mod l10n;
mod link;
mod lock_screen;
mod logging;
mod matrix_login;
mod onboarding;
mod pages;
mod peers;
mod plugins;
mod port_map;
mod push;
mod push_method;
mod scope;
mod servers;
mod session_store;
mod settings;
mod sharing;
mod subscription;
#[cfg(feature = "test-hooks")]
pub(crate) mod test_hooks;
mod widget;

pub use appearance::{accent_hex, default_appearance, parse_accent};
pub use chat::TimelineSubscription;
pub use chat::bot_current_avatar;
pub use client::ClientCore;
pub use files::{FileOpenError, FileOpener};
pub use home::Network;
pub use link::{app_link_url, parse_app_link};
pub use logging::init_logging;
pub use pages::bridge::{
    TabBridgeAction, TabBridgeResult, TabEnvironment, TabPlatform, TabSafeArea,
};
pub use pages::media::{PageMediaAction, PageMediaPlayback, PageMediaState};
pub use pages::media_permission::TabMediaPermission;
pub use sharing::discover_share_source;
pub use subscription::{
    AgentChainsSubscription, AppearanceSubscription, BotContactsSubscription, ChatSubscription,
    ComposerSubscription, ConfirmationsSubscription, FriendsSubscription, HomeSubscription,
    OnboardingSubscription, PagesSubscription, PeersSubscription, PluginsSubscription,
    ServersSubscription, SessionListSubscription, SettingsSubscription, TransfersSubscription,
    UnreadSubscription, WidgetSnapshotSubscription,
};
pub use widget::read_widget_snapshot;
// 协议类型整体转出：并行 PR 各自加名字不再挤在同一段列表里。
pub use wonton_protocol::*;
