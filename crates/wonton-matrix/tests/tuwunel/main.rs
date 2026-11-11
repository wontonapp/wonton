//! 对真实 Tuwunel 的集成测试。
//!
//! 默认每个测试用 Docker 起一台一次性的 Tuwunel，配置与 deploy/dev/matrix 相同，所以需要 Docker。
//! 消息测试也可以连现成的服务器（例如经隧道），见 deploy/dev/matrix/README.md。
//!
//! 这里的测试都标了 `#[ignore]`：普通的 `cargo test` 只列出它们、不运行，没有 Docker 的机器也能通过。
//! 运行：`cargo test -p wonton-matrix --test tuwunel -- --include-ignored`。`scripts/check.sh` 在有
//! Docker 时自动运行，CI 的 Rust 作业也会运行。新增的测试同样要标 `#[ignore]`。

// 同库本身：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

mod chat;
mod client_lifetime;
mod egress;
mod encryption;
mod friends;
mod layout;
mod login;
mod messaging;
mod profile;
mod push;
mod recovery_order;
mod restore_beside_old;
mod send_queue;
mod state;
mod support;
mod thread;
