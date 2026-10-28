//! 客户端厚核心：状态、业务逻辑、网络与 ViewModel，经 UniFFI 导出给 SwiftUI 与 Compose。
//!
//! 接口形态是“操作进、ViewModel 出”，并保持可序列化，以便测试和将来的 Web 端复用。
//! 设计见 `docs/adr/0005-native-clients-thick-core.md`。
//!
//! - 操作进：壳调用 [`ClientCore::dispatch`]，参数是 [`Action`]。
//! - ViewModel 出：壳调用 [`ClientCore::subscribe_session_list`]，循环等待
//!   [`SessionListSubscription::next`]，包装成 Swift 的 `AsyncSequence` 或 Kotlin 的 `Flow`。
//!
//! 数据目前只放在内存里；接入 Matrix 后改由 `wonton-matrix` 同步。

uniffi::setup_scaffolding!();

mod client;
mod session_store;
mod subscription;

pub use client::ClientCore;
pub use subscription::SessionListSubscription;
pub use wonton_protocol::{Action, CoreError, SessionListViewModel, SessionRow};
