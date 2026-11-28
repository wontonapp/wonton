//! 插件包：一个带 `x-wonton` 扩展字段的 compose 文件，加上它引用的文件（图标、设置表单、Bot 头像和
//! 顶层 `configs` 的 `file`，见 [`super::package_files`]）。
//!
//! 解析分两步：[`PluginPackage::parse`]、[`PluginPackage::with_files`]、[`PluginPackage::load_dir`]
//! 只看结构、清单和被引用的文件，不依赖安装环境；[`PluginPackage::services`] 在给定变量表后做插值，
//! 再按 Compose 子集解析每个服务。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::os::fd::AsFd;
use std::path::Path;

use serde::Deserialize;
// `serde_saphyr::Error::custom` 是 serde 的 trait 方法，路径经它带出反序列化器闭包。
use serde::de::Error as _;
use serde_json::{Map, Value};

use super::compose::{
    NetworkMode, forbidden_service_key_reason, forbidden_top_key_reason, network_mode_refusal,
    strip_extensions,
};
use super::github_source::Origin;
use super::images::IconFormat;
use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, ManifestExt};
use super::manifest_static::StaticWebManifestExt;
use super::package_archive;
use super::package_files::{
    self, COMPOSE_FILE, FileError, MAX_FILE_BYTES, MAX_PACKAGE_BYTES, PackageFiles,
};
use crate::l10n::text;

mod bot_service;
#[path = "package_capability_services.rs"]
mod capability_services;
mod configs;
mod gpu_images;
mod references;
mod runtime;
mod services;
pub mod static_files;
mod static_web;

pub(super) mod errors;

pub use configs::ConfigSource;
pub use errors::PackageError;
use errors::{
    archive_error, compose_file_error, file_error, interpolate_error, reference_error,
    service_error, static_error, too_big_error, top_level_error, yaml_error,
};

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

impl PluginPackage {
    /// 只有 compose 文本的插件包：引用了包里的文件时失败。
    pub fn parse(source: &str, langs: &Languages) -> Result<Self, PackageError> {
        Self::with_files(source, PackageFiles::new(), langs)
    }

    /// compose 文本加上被引用的文件（如安装记录里保存的那些）。`files` 里没被引用的文件不保留。
    pub fn with_files(
        source: &str,
        files: PackageFiles,
        langs: &Languages,
    ) -> Result<Self, PackageError> {
        Self::structure(source, langs)?.attach(files, langs)
    }

    /// 插件包打成的压缩包，规则见 [`package_archive`]：只解压 `docker-compose.yml` 和它引用的文件。
    pub fn from_archive(body: &[u8], langs: &Languages) -> Result<Self, PackageError> {
        let archive = package_archive::Archive::open(body).map_err(|e| archive_error(langs, e))?;
        let root = archive.root(langs).map_err(|e| archive_error(langs, e))?;
        let compose = archive
            .read(&root, &[COMPOSE_FILE.to_owned()], langs)
            .map_err(|e| archive_error(langs, e))?
            .remove(COMPOSE_FILE)
            .ok_or_else(|| compose_file_error(langs, FileError::Missing))?;
        let source = String::from_utf8(compose)
            .map_err(|_| yaml_error(langs, text(langs, "plugin-install-pkg-not-utf8", &[])))?;
        let files = archive
            .read(&root, &Self::referenced_files(&source, langs)?, langs)
            .map_err(|e| archive_error(langs, e))?;
        let package = Self::with_files(&source, files, langs)?;
        let Some(static_root) = package.static_root() else {
            return Ok(package);
        };
        let prefix = format!("{root}{static_root}/");
        let files = archive
            .read_tree(&prefix, static_files::MAX_FILE_BYTES, langs)
            .map_err(|e| archive_error(langs, e))?;
        package.with_static(files, langs)
    }

    /// 读插件包目录：`docker-compose.yml` 和它引用的文件，路径和软链接的规则见 [`super::package_files`]。
    pub fn load_dir(dir: &Path, langs: &Languages) -> Result<Self, PackageError> {
        let opened =
            super::package_files::open_dir(dir).map_err(|e| compose_file_error(langs, e))?;
        Self::load_dir_at(opened.as_fd(), langs)
    }

    /// 同 [`Self::load_dir`]，从已打开的目录读：调用方（如 outbox 的解析）钉住目录，中途换不掉。
    pub fn load_dir_at(
        dir: std::os::fd::BorrowedFd<'_>,
        langs: &Languages,
    ) -> Result<Self, PackageError> {
        let source =
            package_files::read_at(dir, COMPOSE_FILE).map_err(|e| compose_file_error(langs, e))?;
        let source = String::from_utf8(source)
            .map_err(|_| yaml_error(langs, text(langs, "plugin-install-pkg-not-utf8", &[])))?;
        let package = Self::structure(&source, langs)?;
        let mut files = PackageFiles::new();
        let optional = package.optional_references(langs)?;
        let required = package.references(langs)?;
        for (field, path) in required.iter().chain(&optional) {
            let data = match package_files::read_at(dir, path) {
                Ok(data) => data,
                Err(FileError::Missing) if optional.iter().any(|(_, p)| p == path) => continue,
                Err(source) => return Err(file_error(langs, field, path, source)),
            };
            files.insert(path.clone(), data);
        }
        let package = package.attach(files, langs)?;
        match package.static_root() {
            Some(root) => {
                let files = static_files::read_dir(dir, &root)
                    .map_err(|problem| static_error(langs, &root, &problem))?;
                package.with_static(files, langs)
            }
            None => Ok(package),
        }
    }

    /// compose 文本里的清单，不管它引用的文件：商店列表只要清单。
    pub fn manifest_of(source: &str, langs: &Languages) -> Result<Manifest, PackageError> {
        Ok(Self::structure(source, langs)?.manifest)
    }

    /// 包里的引用路径，含可选的状态头像首帧；缺少首帧可以省略，其他引用都必须存在。
    pub fn referenced_files(source: &str, langs: &Languages) -> Result<Vec<String>, PackageError> {
        let package = Self::structure(source, langs)?;
        let refs = package.references(langs)?;
        Ok(refs
            .into_iter()
            .chain(package.optional_references(langs)?)
            .map(|(_, path)| path)
            .collect())
    }

    pub fn optional_files(source: &str, langs: &Languages) -> Result<Vec<String>, PackageError> {
        Ok(Self::structure(source, langs)?
            .optional_references(langs)?
            .into_iter()
            .map(|(_, path)| path)
            .collect())
    }

    fn structure(source: &str, langs: &Languages) -> Result<Self, PackageError> {
        let document: Value =
            serde_saphyr::from_str(source).map_err(|e| yaml_error(langs, e.to_string()))?;
        let Value::Object(mut top) = document else {
            return Err(PackageError::NotAMapping(text(
                langs,
                "plugin-install-pkg-not-mapping",
                &[],
            )));
        };
        check_top_level_keys(&top, langs)?;
        let manifest_value = top.remove("x-wonton").ok_or_else(|| {
            PackageError::MissingManifest(text(langs, "plugin-install-pkg-missing-manifest", &[]))
        })?;
        super::store_info::check_spec(&manifest_value, langs)?;
        let mut manifest: Manifest = match serde_json::from_value(manifest_value) {
            Ok(manifest) => manifest,
            Err(e) => {
                // 两种解析对个别标量的松紧不同（`version: 1.0` 直接反序列化能当成字符串），成功与否仍以
                // serde_json 的结果为准；重解析失败才借它的报错写清字段和行号。
                let inner = manifest_from_yaml(source)
                    .err()
                    .unwrap_or_else(|| ManifestError::Invalid(e.to_string()));
                return Err(PackageError::Manifest(ManifestError::Invalid(text(
                    langs,
                    "plugin-install-pkg-manifest",
                    &[("error", inner.to_string().into())],
                ))));
            }
        };
        manifest.imply_static_webui();
        manifest.validate(langs)?;

        let services = runtime::services(top.remove("services"), manifest.runtime, langs)?;
        for (name, service) in &services {
            check_service_name(name, &manifest, langs)?;
            check_service_keys(name, service, langs)?;
        }
        let volumes = parse_named_volumes(top.remove("volumes"), langs)?;
        let configs = configs::parse_configs(top.remove("configs"), langs)?;
        let package = Self {
            manifest,
            source: source.to_owned(),
            services,
            volumes,
            configs,
            files: PackageFiles::new(),
            static_files: None,
            static_hashes: static_files::StaticHashes::new(),
            origin: None,
        };
        package.check_manifest_references(langs)?;
        Ok(package)
    }

    /// 被引用的文件：（引用它的字段，规范化后的路径）。
    fn references(&self, langs: &Languages) -> Result<Vec<(String, String)>, PackageError> {
        let manifest = &self.manifest;
        let mut refs = Vec::new();
        let mut add = |field: &str, raw: &str| {
            let path = package_files::normalize(raw)
                .map_err(|message| reference_error(langs, field, message))?;
            refs.push((field.to_owned(), path));
            Ok::<_, PackageError>(())
        };
        let mut image_refs = Vec::new();
        if let Some(icon) = &manifest.icon {
            image_refs.push(("icon".to_owned(), icon.as_str()));
        }
        for (index, tab) in manifest
            .tabs
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            image_refs.extend(tab.icon_refs(index));
        }
        for (field, icon) in image_refs {
            // 图标随提交固定，App 也不向第三方取图（ADR-0017），所以不收网址。
            if IconFormat::from_path(icon).is_none() || icon.contains("://") {
                return Err(reference_error(
                    langs,
                    &field,
                    text(
                        langs,
                        "plugin-install-pkg-icon-file",
                        &[("icon", icon.into())],
                    ),
                ));
            }
            add(&field, icon)?;
        }
        if let Some(settings) = &manifest.settings {
            add("settings", settings)?;
        }
        if let Some(avatar) = manifest.bot.as_ref().and_then(|b| b.avatar.as_ref()) {
            add("bot.avatar", avatar)?;
        }
        if let Some(avatars) = manifest.bot.as_ref().and_then(|b| b.avatars.as_ref()) {
            for (state, path) in avatars {
                add(&format!("bot.avatars.{state}"), path)?;
            }
        }
        for notice in &manifest.notices {
            add("notices", notice)?;
        }
        for (name, config) in &self.configs {
            if let ConfigSource::File(path) = config {
                refs.push((format!("configs.{name}.file"), path.clone()));
            }
        }
        Ok(refs)
    }

    fn optional_references(
        &self,
        langs: &Languages,
    ) -> Result<Vec<(String, String)>, PackageError> {
        let required: BTreeSet<_> = self
            .references(langs)?
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        let mut optional = BTreeMap::new();
        if let Some(avatars) = self
            .manifest
            .bot
            .as_ref()
            .and_then(|bot| bot.avatars.as_ref())
        {
            for (state, raw) in avatars {
                let path = package_files::normalize(raw).map_err(|message| {
                    reference_error(langs, &format!("bot.avatars.{state}"), message)
                })?;
                let poster = super::bot_avatars::poster_path(&path);
                if poster != path && !required.contains(&poster) {
                    optional.insert(poster, format!("bot.avatars.{state}.poster"));
                }
            }
        }
        Ok(optional
            .into_iter()
            .map(|(path, field)| (field, path))
            .collect())
    }

    /// 配上被引用的文件：每个都要有，且不超过大小上限。
    fn attach(mut self, mut files: PackageFiles, langs: &Languages) -> Result<Self, PackageError> {
        let optional = self.optional_references(langs)?;
        let required = self.references(langs)?;
        for (field, path) in required.iter().chain(&optional) {
            if self.files.contains_key(path) {
                continue;
            }
            let Some(data) = files.remove(path) else {
                if optional.iter().any(|(_, p)| p == path) {
                    continue;
                }
                return Err(file_error(langs, field, path, FileError::Missing));
            };
            if data.len() as u64 > MAX_FILE_BYTES {
                return Err(file_error(langs, field, path, FileError::TooBig));
            }
            if field == "notices" && std::str::from_utf8(&data).is_err() {
                return Err(file_error(langs, field, path, FileError::NotText));
            }
            self.files.insert(path.clone(), data);
        }
        let total = self.source.len() + self.files.values().map(Vec::len).sum::<usize>();
        if total as u64 > MAX_PACKAGE_BYTES {
            return Err(too_big_error(langs, total as u64));
        }
        Ok(self)
    }

    /// 插件包打成的 tar 归档，[`Self::from_archive`] 读得回来。
    pub fn archive(&self) -> Vec<u8> {
        let mut files = self.files.clone();
        files.extend(
            self.static_blobs()
                .map(|(path, data)| (path, data.to_vec())),
        );
        package_files::write_archive(self.source.as_bytes(), &files)
    }

    /// 插件包摘要，见 [`package_files::digest`]：静态网页的文件也算在内，按包里的路径。
    pub fn digest(&self) -> String {
        let root = self.static_root().unwrap_or_default();
        let hashes = self
            .static_hashes
            .iter()
            .map(|(path, hash)| (format!("{root}/{path}"), hash.clone()));
        package_files::digest_with(self.source.as_bytes(), &self.files, hashes)
    }

    /// 被引用的文件，安装时随记录保存。
    pub fn files(&self) -> &PackageFiles {
        &self.files
    }

    /// 清单 `x-wonton.settings` 指向的设置 Schema 原文；没声明时为空（引用在解析包时已核对存在并是 UTF-8）。
    pub fn settings(&self) -> Option<&[u8]> {
        let path = self.manifest.settings.as_deref()?;
        self.files.get(path).map(Vec::as_slice)
    }
}

/// serde 报错带字段路径（如 `webui.port`）：路径在根上时没有可写的段，原样返回信息。
fn pathed<E: fmt::Display>(error: &serde_path_to_error::Error<E>) -> String {
    let path = error.path().to_string();
    if path.is_empty() {
        error.inner().to_string()
    } else {
        format!("`{path}`: {}", error.inner())
    }
}

/// 从 YAML 文本直接反序列化清单：`serde_json::from_value` 丢掉了位置，失败时用它把报错写到
/// 字段和行号（如 `x-wonton.description.Y`，加 serde-saphyr 给的 at line N, column M）。
pub(crate) fn manifest_from_yaml(source: &str) -> Result<Manifest, ManifestError> {
    #[derive(Deserialize)]
    struct ManifestDocument {
        #[serde(rename = "x-wonton")]
        manifest: Manifest,
    }
    serde_saphyr::with_deserializer_from_str(source, |de| {
        // serde-saphyr 只在闭包里出借反序列化器，错误类型固定是它自己的，路径经 custom 带出去。
        serde_path_to_error::deserialize::<_, ManifestDocument>(de)
            .map_err(|e| serde_saphyr::Error::custom(pathed(&e)))
    })
    .map(|document| document.manifest)
    .map_err(|e| ManifestError::Invalid(e.to_string()))
}

fn check_top_level_keys(top: &Map<String, Value>, langs: &Languages) -> Result<(), PackageError> {
    for key in top.keys() {
        if let Some(reason) = forbidden_top_key_reason(key, langs) {
            return Err(top_level_error(langs, key, reason));
        }
        let known = matches!(
            key.as_str(),
            "name" | "version" | "services" | "volumes" | "configs"
        );
        if !known && !key.starts_with("x-") {
            return Err(top_level_error(
                langs,
                key,
                text(langs, "plugin-install-pkg-not-in-subset", &[]),
            ));
        }
    }
    Ok(())
}

/// 服务名同时用作网络别名和容器名的一部分，限制为小写 DNS 安全字符。
fn check_service_name(
    name: &str,
    manifest: &Manifest,
    langs: &Languages,
) -> Result<(), PackageError> {
    if super::host_relay::reserved_service_name(manifest, name) {
        return Err(service_error(
            langs,
            name,
            text(langs, "plugin-install-pkg-reserved-service", &[]),
        ));
    }
    let valid = !name.is_empty()
        && name.len() <= 63
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return Err(service_error(
            langs,
            name,
            text(langs, "plugin-install-pkg-service-name-dns", &[]),
        ));
    }
    // 服务名会拼进网络别名 `<服务名>.<slug>.wonton.internal`（核心连上游的 Host，见 [`super::names`]），
    // 结尾不能是 `-`；Bot 服务的别名是 `<服务名>-default`。带 `--` 的裸服务名别名可以和别的插件的容器名
    // 一样，抢走发给它的请求。
    if name.ends_with('-') || name.contains("--") {
        return Err(service_error(
            langs,
            name,
            text(langs, "plugin-install-pkg-service-name-dashes", &[]),
        ));
    }
    Ok(())
}

fn check_service_keys(name: &str, service: &Value, langs: &Languages) -> Result<(), PackageError> {
    let service_err = |message: String| service_error(langs, name, message);
    let Value::Object(map) = service else {
        return Err(service_err(
            "the service definition must be a mapping".to_owned(),
        ));
    };
    super::network_mode::validate_fields(name, map)?;
    for key in map.keys() {
        if let Some(reason) = forbidden_service_key_reason(key, langs) {
            return Err(service_err(format!("unsupported key `{key}`: {reason}")));
        }
    }
    match map.get("network_mode") {
        None => Ok(()),
        Some(Value::String(mode)) => match NetworkMode::try_from(mode.clone()) {
            Ok(_) => Ok(()),
            Err(_) => Err(service_err(format!(
                "unsupported network_mode `{mode}`: {}",
                network_mode_refusal(mode, langs)
            ))),
        },
        Some(other) => Err(service_err(format!(
            "network_mode must be a string, not `{other}`"
        ))),
    }
}

fn parse_named_volumes(
    value: Option<Value>,
    langs: &Languages,
) -> Result<BTreeSet<String>, PackageError> {
    let Some(value) = value else {
        return Ok(BTreeSet::new());
    };
    let Value::Object(map) = value else {
        return Err(PackageError::Volume(text(
            langs,
            "plugin-install-pkg-volume",
            &[("name", "volumes".into())],
        )));
    };
    map.into_iter()
        .map(|(name, mut definition)| {
            strip_extensions(&mut definition);
            let empty = match &definition {
                Value::Null => true,
                Value::Object(fields) => fields.is_empty(),
                _ => false,
            };
            let name_ok = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if empty && name_ok {
                Ok(name)
            } else {
                Err(PackageError::Volume(text(
                    langs,
                    "plugin-install-pkg-volume",
                    &[("name", name.as_str().into())],
                )))
            }
        })
        .collect()
}
