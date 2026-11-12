//! 第三方客户端的公网入口（ADR-0021、ADR-0089、ADR-0090）。这里按职责放小模块，入口容器和网关的接线不在这里。
//!
//! `domain` 是自己的域名和运行时签发；`providers` 是服务商预设；`path_rules` 是公开路径的校验。

pub mod domain;
pub mod path_rules;
#[cfg(test)]
mod domain_docker_tests;
#[cfg(test)]
mod domain_tests;
#[cfg(test)]
mod path_rules_tests;
pub mod providers;
pub(crate) mod state;
