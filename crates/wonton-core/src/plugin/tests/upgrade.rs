//! 升级（ADR-0017）：查到新版本，保留数据和 Bot 的设备，只换容器；新版本装不好就退回旧版本，错误如实报。
//! 按升级阶段拆在子模块里：`offers` 查新版本和给主人的提示，`confirm` 确认页和预览，
//! `apply` 执行升级和失败后退回，`system` 是随核心带的和系统插件的升级；共用夹具在 `support`。

mod apply;
mod capability_deps;
mod confirm;
mod offers;
mod support;
mod system;

// 测试函数由测试框架按定义注册，编译器看不到这些重导出的使用；留着保住旧路径 tests::upgrade::<测试名>。
#[allow(unused_imports)]
pub(super) use apply::*;
#[allow(unused_imports)]
pub(super) use confirm::*;
#[allow(unused_imports)]
pub(super) use offers::*;
pub(super) use support::*;
#[allow(unused_imports)]
pub(super) use system::*;

pub(super) use std::fs;
pub(super) use std::sync::Arc;

pub(super) use serde_json::{Value, json};
pub(super) use tokio::sync::watch;
pub(super) use wonton_protocol::{InstallRequest, PluginState, PluginTrust, UpgradeRequest};

pub(super) use super::super::fake_docker::FakeDocker;
pub(super) use super::super::names::container_name;
pub(super) use super::super::preinstall::Backoff;
pub(super) use super::super::storefront::{StoreConfig, index::IndexEntry};
pub(super) use super::super::{
    InstallError, PluginId, PluginManager, PluginPackage, PluginStatus, UpgradeTo,
};
pub(super) use super::super::{github::GitHub, manifest::Permission};
pub(super) use super::lan::{office, on};
pub(super) use super::manager::{manager_in_container, manager_with};
pub(super) use super::{egress, langs, message};
pub(super) use super::{
    env_of_container,
    github::{FakeGitHub, zh},
};
