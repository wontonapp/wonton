//! 客户端厚核心：状态、业务逻辑、网络与 ViewModel，经 UniFFI 导出给 SwiftUI 与 Compose。
//!
//! 接口形态是“操作进、ViewModel 出”，并保持可序列化，以便测试和将来的 Web 端复用。
//! 设计见 `docs/adr/0005-native-clients-thick-core.md`。
//!
//! - 操作进：壳调用 [`ClientCore::dispatch`]，参数是 [`Action`]。
//! - ViewModel 出：壳调用 [`ClientCore::subscribe_session_list`]、[`ClientCore::subscribe_home`]，
//!   循环等待订阅的 `next()`，包装成 Swift 的 `AsyncSequence` 或 Kotlin 的 `Flow`。
//!
//! 会话数据目前只放在内存里，接入 Matrix 后改由 `wonton-matrix` 同步。和家里服务器的连接见 `home`
//! 模块：iroh 隧道加 WebView 用的本地代理（ADR-0004）。

uniffi::setup_scaffolding!();

mod client;
mod home;
mod logging;
mod session_store;
mod subscription;

pub use client::ClientCore;
pub use home::Network;
pub use logging::init_logging;
pub use subscription::{HomeSubscription, SessionListSubscription};
pub use wonton_protocol::{
    Action, CoreError, HomeFailure, HomePath, HomeStatus, HomeViewModel, ProxySettings,
    SessionListViewModel, SessionRow,
};
