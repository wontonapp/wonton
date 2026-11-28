//! 假扮的核心管理接口和驱动 App 的工具，只走回环地址。

use std::time::Duration;

mod answers;
mod bot_activity;
mod driver;
mod fixtures;
mod home;
mod router;
mod state;

pub use answers::{Canned, Headered, core_error};
pub use driver::Rig;
pub use fixtures::{installed, item, listing, preview, running};
pub use home::{FakeHome, ticket};
pub use router::fake_core;
#[allow(unused_imports)]
// `Seen` 只在 `rig::Seen` 这一层露出，用例都从 `seen()` 的返回值推断。
pub use state::{CoreState, Seen};

pub const WAIT: Duration = Duration::from_secs(20);
/// 兜底预算，不是节奏：繁忙时建连实测 16s+（登录令牌两次超时重连），单次建连限时 12s，
/// 失败后按退避重试；取 60s，超过就是真的连不上。
pub const CONNECT_BACKSTOP: Duration = Duration::from_secs(60);
pub const IMAGE: &str = "/_wonton/api/images/abc/icon-192.png";
