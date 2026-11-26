//! 模型网关：在假 Docker 和假网关上验证凭据的签发、验证、对账和吊销，以及注入给使用方的变量。假网关的凭据命令
//! 按 `plugins/magpie/credentials.sh` 的约定：`issue` 已有同名凭据时不改动，`revoke` 本来没有也算成功。
//! 按行为拆在子模块里：`injection` 签发和注入使用方变量，`lifecycle` 装、卸、到达和恢复网关，
//! `reconcile` 对账失败和命令结局未知后的恢复，`wants` 用 `wants: [models]` 声明；共用夹具在 `support`。

mod injection;

mod support;

// 测试函数由测试框架按定义注册，编译器看不到这些重导出的使用；留着保住旧路径 tests::models::<测试名>。
#[allow(unused_imports)]
pub(super) use injection::*;
pub(super) use support::*;

pub(super) use std::collections::BTreeMap;
pub(super) use std::sync::{Arc, Mutex};

pub(super) use axum::Router;
pub(super) use axum::http::{HeaderMap, StatusCode};
pub(super) use axum::routing::get;
pub(super) use tokio::net::TcpListener;

pub(super) use super::super::fake_docker::FakeDocker;
pub(super) use super::super::names::Instance;
pub(super) use super::super::names::container_name;
pub(super) use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
pub(super) use super::github::zh;
pub(super) use super::langs;
pub(super) use super::manager::manager_with;
pub(super) use super::{env_of_container, package};
pub(super) use crate::http::HttpClient;
pub(super) use crate::matrix::Matrix;
