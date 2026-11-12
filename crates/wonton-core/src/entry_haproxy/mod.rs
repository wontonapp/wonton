//! 入口 HAProxy 的配置和标签：输入一个模型（联邦名字、打开了的标签、来源怎么来），输出配置文本。
//!
//! 写进配置的值都先过这里的校验，不过就整体报错，不生成半份配置。

pub mod labels;
pub mod model;
pub mod render;

pub use labels::{PROBE_LABEL, RESERVED, assign, ensure, validate_label, validate_name};
pub use model::{
    AUTH_FAILURE_HEADER, AUTH_FAILURES_LIMIT, CERT_FILE, CONNECTOR_ADDRESS, EDGE_SUBNET,
    ENTRY_EDGE_ADDRESS, GATEWAY_ADDRESS, LAN_BIND, LAN_CRT_LIST, LAN_FORWARD_ADDRESS,
    LAN_FORWARD_SUBNET, Model, PAIRED_BIND, PUBLIC_BIND, PUBLIC_CRT_LIST, RUNTIME_SOCKET, Trust,
    validate_header_name, validate_peer, validate_suffix,
};
pub use render::{Rendered, render};

#[cfg(test)]
mod docker_tests;
#[cfg(test)]
mod label_tests;
#[cfg(test)]
mod render_tests;
