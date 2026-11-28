//! 插件包：一个带 `x-wonton` 扩展字段的 compose 文件，加上它引用的文件（图标、设置表单、Bot 头像和
//! 顶层 `configs` 的 `file`，见 [`super::package_files`]）。
//!
//! 解析分两步：[`PluginPackage::parse`]、[`PluginPackage::with_files`]、[`PluginPackage::load_dir`]
//! 只看结构、清单和被引用的文件，不依赖安装环境；[`PluginPackage::services`] 在给定变量表后做插值，
//! 再按 Compose 子集解析每个服务。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Map, Value};

use super::github_source::Origin;
use super::manifest::Manifest;
use super::package_files::PackageFiles;

mod archive;
mod bot_service;
#[path = "package_capability_services.rs"]
mod capability_services;
mod configs;
mod file_refs;
mod gpu_images;
mod load;
mod references;
mod runtime;
mod services;
pub mod static_files;
mod static_web;
mod structure;

pub(super) mod errors;

pub use configs::ConfigSource;
pub use errors::PackageError;
pub(crate) use structure::manifest_from_yaml;

/// 解析好的插件包。
#[derive(Debug, Clone)]
pub struct PluginPackage {
    pub manifest: Manifest,
    /// 原始 compose 文本，安装后原样保存，便于审计和重装。
    pub source: String,
    services: Map<String, Value>,
    volumes: BTreeSet<String>,
    configs: BTreeMap<String, ConfigSource>,
    files: PackageFiles,
    /// `runtime: static_web` 的静态文件（相对 `static_web.root`），只在装、升级时带着；从安装记录恢复的为空。
    static_files: Option<static_files::StaticFiles>,
    /// 静态文件的 sha256，算插件包摘要用；从安装记录恢复时只有它。
    static_hashes: static_files::StaticHashes,
    /// 从哪里取来的；上传的和预装的插件包为空。
    pub origin: Option<Origin>,
}

/// serde 报错带字段路径（如 `webui.port`）：路径在根上时没有可写的段，原样返回信息。
pub(super) fn pathed<E: fmt::Display>(error: &serde_path_to_error::Error<E>) -> String {
    let path = error.path().to_string();
    if path.is_empty() {
        error.inner().to_string()
    } else {
        format!("`{path}`: {}", error.inner())
    }
}
