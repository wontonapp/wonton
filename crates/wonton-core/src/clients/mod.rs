//! 第三方客户端的公网入口（ADR-0021、ADR-0089、ADR-0090）。这里按职责放小模块，入口容器和网关的接线不在这里。
//!
//! `domain` 是自己的域名和运行时签发；`providers` 是服务商预设；`path_rules` 是公开路径的校验；
//! `credentials` 跑应用里的凭据命令；`pending` 管待完成的签发、换值、吊销和共享密码；`guests` 是访客链接；

pub(crate) mod apply;
pub(crate) mod credentials;
#[cfg(test)]
mod credentials_tests;
pub mod domain;
#[cfg(test)]
mod domain_docker_tests;
pub mod external_url;
pub mod gateway_auth;
pub mod guests;
pub mod lan_forward;
pub mod passwords;
pub mod path_rules;
#[cfg(test)]
mod path_rules_tests;
pub(crate) mod pending;
#[cfg(test)]
mod pending_tests;
pub mod providers;
pub(crate) mod state;
