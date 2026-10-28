//! 对真实 Tuwunel 的集成测试。
//!
//! 默认每个测试用 Docker 起一台一次性的 Tuwunel，配置与 deploy/dev/matrix 相同，所以需要 Docker。
//! 消息测试也可以连现成的服务器（例如经隧道），见 deploy/dev/matrix/README.md。

// 同库本身：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

mod login;
mod messaging;
mod support;
