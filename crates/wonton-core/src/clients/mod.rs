//! 第三方客户端的公网入口（ADR-0021、ADR-0089、ADR-0090）。这里按职责放小模块，入口容器和网关的接线不在这里。

pub mod path_rules;

#[cfg(test)]
mod path_rules_tests;
