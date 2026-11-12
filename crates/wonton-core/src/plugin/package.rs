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
    FORBIDDEN_CONFIG_KEYS, FORBIDDEN_SERVICE_KEYS, FORBIDDEN_TOP_KEYS, Service,
    network_mode_refusal, strip_extensions,
};
use super::github_source::Origin;
use super::images::IconFormat;
use super::interpolate::{InterpolateError, UsedVars, Vars, interpolate, interpolate_value};
use super::manifest::{Manifest, ManifestError, SecretSpec};
use super::package_archive;
use super::package_files::{
    self, COMPOSE_FILE, FileError, MAX_FILE_BYTES, MAX_PACKAGE_BYTES, PackageFiles,
};

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("compose 文件不是合法的 YAML：{0}")]
    Yaml(String),
    #[error("compose 文件顶层必须是映射")]
    NotAMapping,
    #[error("compose 文件缺少 `x-wonton` 扩展字段；没有清单的 compose 需要先补全权限声明")]
    MissingManifest,
    #[error("`x-wonton` 不合法：{0}")]
    Manifest(#[from] ManifestError),
    #[error("顶层字段 `{key}` 不受支持：{reason}")]
    TopLevel { key: String, reason: String },
    #[error("服务 `{service}`：{message}")]
    Service { service: String, message: String },
    #[error("服务 `{service}` 插值失败：{source}")]
    Interpolate {
        service: String,
        #[source]
        source: InterpolateError,
    },
    #[error("{field} 引用的服务 `{service}` 不存在")]
    UnknownService {
        field: &'static str,
        service: String,
    },
    #[error("命名卷 `{0}` 不合法：M0 只支持不带参数的本地卷")]
    Volume(String),
    #[error("配置 `{name}`：{message}")]
    Config { name: String, message: String },
    #[error("配置 `{name}` 插值失败：{source}")]
    ConfigInterpolate {
        name: String,
        #[source]
        source: InterpolateError,
    },
    #[error("{field}：{message}")]
    Reference { field: String, message: String },
    #[error("{field} 引用的文件 `{path}`：{source}")]
    File {
        field: String,
        path: String,
        #[source]
        source: FileError,
    },
    #[error("插件包的 {COMPOSE_FILE}：{0}")]
    ComposeFile(FileError),
    #[error("插件包归档：{0}")]
    Archive(String),
    #[error("插件包一共 {} KiB，超过 {} KiB", .0.div_ceil(1024), MAX_PACKAGE_BYTES >> 10)]
    TooBig(u64),
}

/// 顶层 `configs` 的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// 插件包里的文件（规范化后的相对路径），原样写进容器。
    File(String),
    /// 内联内容，安装时按注入的变量插值。
    Content(String),
}

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
    /// 从哪里取来的；上传的和预装的插件包为空。
    pub origin: Option<Origin>,
}

pub(super) type ServiceDependencies<'a> =
    BTreeMap<&'a str, Vec<(String, super::compose::DependCondition)>>;

impl PluginPackage {
    /// compose 里有没有这个服务。
    pub fn has_service(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }

    /// 只有 compose 文本的插件包：引用了包里的文件时失败。
    pub fn parse(source: &str) -> Result<Self, PackageError> {
        Self::with_files(source, PackageFiles::new())
    }

    /// compose 文本加上被引用的文件（如安装记录里保存的那些）。`files` 里没被引用的文件不保留。
    pub fn with_files(source: &str, files: PackageFiles) -> Result<Self, PackageError> {
        Self::structure(source)?.attach(files)
    }

    /// 插件包打成的压缩包，规则见 [`package_archive`]：只解压 `docker-compose.yml` 和它引用的文件。
    pub fn from_archive(body: &[u8]) -> Result<Self, PackageError> {
        let archive = package_archive::Archive::open(body).map_err(PackageError::Archive)?;
        let root = archive.root().map_err(PackageError::Archive)?;
        let compose = archive
            .read(&root, &[COMPOSE_FILE.to_owned()])
            .map_err(PackageError::Archive)?
            .remove(COMPOSE_FILE)
            .ok_or(PackageError::ComposeFile(FileError::Missing))?;
        let source = String::from_utf8(compose)
            .map_err(|_| PackageError::Yaml("不是 UTF-8 文本".to_owned()))?;
        let files = archive
            .read(&root, &Self::referenced_files(&source)?)
            .map_err(PackageError::Archive)?;
        Self::with_files(&source, files)
    }

    /// 读插件包目录：`docker-compose.yml` 和它引用的文件，路径和软链接的规则见 [`super::package_files`]。
    pub fn load_dir(dir: &Path) -> Result<Self, PackageError> {
        let opened = super::package_files::open_dir(dir).map_err(PackageError::ComposeFile)?;
        Self::load_dir_at(opened.as_fd())
    }

    /// 同 [`Self::load_dir`]，从已打开的目录读：调用方（如 outbox 的解析）钉住目录，中途换不掉。
    pub fn load_dir_at(dir: std::os::fd::BorrowedFd<'_>) -> Result<Self, PackageError> {
        let source =
            package_files::read_at(dir, COMPOSE_FILE).map_err(PackageError::ComposeFile)?;
        let source = String::from_utf8(source)
            .map_err(|_| PackageError::Yaml("不是 UTF-8 文本".to_owned()))?;
        let package = Self::structure(&source)?;
        let mut files = PackageFiles::new();
        for (field, path) in package.references()? {
            let data = package_files::read_at(dir, &path).map_err(|source| PackageError::File {
                field,
                path: path.clone(),
                source,
            })?;
            files.insert(path, data);
        }
        package.attach(files)
    }

    /// compose 文本里的清单，不管它引用的文件：商店列表只要清单。
    pub fn manifest_of(source: &str) -> Result<Manifest, PackageError> {
        Ok(Self::structure(source)?.manifest)
    }

    /// compose 文本引用的插件包里的文件（规范化后的路径）：按来源取插件包时，取完 compose 再取它们。
    pub fn referenced_files(source: &str) -> Result<Vec<String>, PackageError> {
        let refs = Self::structure(source)?.references()?;
        Ok(refs.into_iter().map(|(_, path)| path).collect())
    }

    fn structure(source: &str) -> Result<Self, PackageError> {
        let document: Value =
            serde_saphyr::from_str(source).map_err(|e| PackageError::Yaml(e.to_string()))?;
        let Value::Object(mut top) = document else {
            return Err(PackageError::NotAMapping);
        };
        check_top_level_keys(&top)?;
        let manifest_value = top
            .remove("x-wonton")
            .ok_or(PackageError::MissingManifest)?;
        super::store_info::check_spec(&manifest_value)?;
        let manifest: Manifest = match serde_json::from_value(manifest_value) {
            Ok(manifest) => manifest,
            Err(e) => {
                // 两种解析对个别标量的松紧不同（`version: 1.0` 直接反序列化能当成字符串），成功与否仍以
                // serde_json 的结果为准；重解析失败才借它的报错写清字段和行号。
                return Err(manifest_from_yaml(source)
                    .err()
                    .unwrap_or_else(|| ManifestError::Invalid(e.to_string()))
                    .into());
            }
        };
        manifest.validate()?;

        let services = match top.remove("services") {
            Some(Value::Object(services)) if !services.is_empty() => services,
            _ => {
                return Err(PackageError::Service {
                    service: "*".to_owned(),
                    message: "至少需要一个服务".to_owned(),
                });
            }
        };
        for (name, service) in &services {
            check_service_name(name)?;
            check_service_keys(name, service)?;
        }
        let volumes = parse_named_volumes(top.remove("volumes"))?;
        let configs = parse_configs(top.remove("configs"))?;
        let package = Self {
            manifest,
            source: source.to_owned(),
            services,
            volumes,
            configs,
            files: PackageFiles::new(),
            origin: None,
        };
        package.check_manifest_references()?;
        Ok(package)
    }

    /// 被引用的文件：（引用它的字段，规范化后的路径）。
    fn references(&self) -> Result<Vec<(String, String)>, PackageError> {
        let manifest = &self.manifest;
        let mut refs = Vec::new();
        let mut add = |field: &str, raw: &str| {
            let path =
                package_files::normalize(raw).map_err(|message| PackageError::Reference {
                    field: field.to_owned(),
                    message,
                })?;
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
                return Err(PackageError::Reference {
                    field,
                    message: format!("`{icon}` 要是插件包里的 PNG 或 SVG 文件"),
                });
            }
            add(&field, icon)?;
        }
        if let Some(settings) = &manifest.settings {
            add("settings", settings)?;
        }
        if let Some(avatar) = manifest.bot.as_ref().and_then(|b| b.avatar.as_ref()) {
            add("bot.avatar", avatar)?;
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

    /// 配上被引用的文件：每个都要有，且不超过大小上限。
    fn attach(mut self, mut files: PackageFiles) -> Result<Self, PackageError> {
        for (field, path) in self.references()? {
            if self.files.contains_key(&path) {
                continue;
            }
            let file_err = |source| PackageError::File {
                field: field.clone(),
                path: path.clone(),
                source,
            };
            let data = files
                .remove(&path)
                .ok_or_else(|| file_err(FileError::Missing))?;
            if data.len() as u64 > MAX_FILE_BYTES {
                return Err(file_err(FileError::TooBig));
            }
            if field == "notices" && std::str::from_utf8(&data).is_err() {
                return Err(file_err(FileError::NotText));
            }
            self.files.insert(path, data);
        }
        let total = self.source.len() + self.files.values().map(Vec::len).sum::<usize>();
        if total as u64 > MAX_PACKAGE_BYTES {
            return Err(PackageError::TooBig(total as u64));
        }
        Ok(self)
    }

    /// 插件包打成的 tar 归档，[`Self::from_archive`] 读得回来。
    pub fn archive(&self) -> Vec<u8> {
        package_files::write_archive(self.source.as_bytes(), &self.files)
    }

    /// 插件包摘要，见 [`package_files::digest`]。
    pub fn digest(&self) -> String {
        package_files::digest(self.source.as_bytes(), &self.files)
    }

    /// 服务的 `environment` 的原文（没插值）：引用了哪些变量、写了什么字面值。
    pub fn raw_environment(&self, service: &str) -> String {
        let environment = self.services.get(service).map(|raw| &raw["environment"]);
        environment.map(ToString::to_string).unwrap_or_default()
    }

    pub fn service_names(&self) -> impl Iterator<Item = &str> {
        self.services.keys().map(String::as_str)
    }

    /// 启动现存容器只需要依赖条件，不重新插值环境或改写容器配置。
    pub(super) fn service_dependencies(&self) -> Result<ServiceDependencies<'_>, PackageError> {
        self.services
            .iter()
            .map(|(name, service)| {
                let dependencies = service
                    .get("depends_on")
                    .map(|value| {
                        serde_path_to_error::deserialize::<_, super::compose::DependsOn>(
                            value.clone(),
                        )
                    })
                    .transpose()
                    .map_err(|e| PackageError::Service {
                        service: name.clone(),
                        message: pathed(&e),
                    })?;
                Ok((
                    name.as_str(),
                    dependencies.map(|deps| deps.entries()).unwrap_or_default(),
                ))
            })
            .collect()
    }

    pub fn named_volumes(&self) -> &BTreeSet<String> {
        &self.volumes
    }

    /// 一次性服务：写了 `restart: "no"` 的服务，和被其他服务以 `condition: service_completed_successfully`
    /// 依赖的服务（后者没写 `restart: "no"` 时渲染报错）。
    pub fn one_shot_services(&self) -> BTreeSet<&str> {
        let completed = Value::String("service_completed_successfully".to_owned());
        let no_restart = Value::String("no".to_owned());
        let depended_on = self
            .services
            .values()
            .filter_map(|service| service.get("depends_on")?.as_object())
            .flat_map(|deps| deps.iter())
            .filter(|(_, spec)| spec.get("condition") == Some(&completed))
            .filter_map(|(dep, _)| self.services.get_key_value(dep).map(|(k, _)| k.as_str()));
        self.services
            .iter()
            .filter(|(_, service)| service.get("restart") == Some(&no_restart))
            .map(|(name, _)| name.as_str())
            .chain(depended_on)
            .collect()
    }

    /// 被引用的文件，安装时随记录保存。
    pub fn files(&self) -> &PackageFiles {
        &self.files
    }

    pub fn has_config(&self, name: &str) -> bool {
        self.configs.contains_key(name)
    }

    /// 配置写进容器的内容：`file` 是包里的文件原样，`content` 按变量表插值。
    pub fn config_data(
        &self,
        name: &str,
        vars: &Vars,
        used: &mut UsedVars,
    ) -> Result<Vec<u8>, PackageError> {
        let config_err = |message: String| PackageError::Config {
            name: name.to_owned(),
            message,
        };
        match self.configs.get(name) {
            None => Err(config_err("没有在顶层 configs 中声明".to_owned())),
            Some(ConfigSource::File(path)) => self
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| config_err(format!("插件包里没有 `{path}`"))),
            Some(ConfigSource::Content(text)) => interpolate(text, vars, used)
                .map(String::into_bytes)
                .map_err(|source| PackageError::ConfigInterpolate {
                    name: name.to_owned(),
                    source,
                }),
        }
    }

    /// 插值后按 Compose 子集解析所有服务。`vars` 给出每个服务用的变量表：Bot 服务比全插件一份的服务
    /// 多几个变量。
    pub fn services<'v>(
        &self,
        vars: impl Fn(&str) -> &'v Vars,
        used: &mut BTreeMap<String, UsedVars>,
    ) -> Result<BTreeMap<String, Service>, PackageError> {
        let services: BTreeMap<String, Service> = self
            .services
            .iter()
            .map(|(name, raw)| {
                let mut value = raw.clone();
                strip_extensions(&mut value);
                let mut variables = UsedVars::new();
                interpolate_value(&mut value, vars(name), &mut variables).map_err(|source| {
                    PackageError::Interpolate {
                        service: name.clone(),
                        source,
                    }
                })?;
                let service: Service =
                    serde_path_to_error::deserialize(value).map_err(|e| PackageError::Service {
                        service: name.clone(),
                        message: pathed(&e),
                    })?;
                if !variables.is_empty() {
                    used.insert(name.clone(), variables);
                }
                Ok((name.clone(), service))
            })
            .collect::<Result<_, PackageError>>()?;
        Ok(services)
    }

    fn check_manifest_references(&self) -> Result<(), PackageError> {
        let models = self
            .manifest
            .provides
            .as_ref()
            .and_then(|p| p.models.as_ref());
        let credentials = models.and_then(|m| m.credentials.service.as_ref());
        let egress = self
            .manifest
            .provides
            .as_ref()
            .and_then(|p| p.egress_proxy.as_ref());
        let refs = [
            (
                "webui.service",
                self.manifest.webui.as_ref().map(|w| &w.service),
            ),
            (
                "mcp.service",
                self.manifest.mcp.as_ref().map(|m| &m.service),
            ),
            ("provides.models.service", models.map(|m| &m.service)),
            ("provides.models.credentials.service", credentials),
            ("provides.egress_proxy.service", egress.map(|p| &p.service)),
        ];
        let collects = self
            .manifest
            .secrets
            .values()
            .filter_map(|spec| match spec {
                SecretSpec::Collect(collect) => {
                    Some(("secrets 的 collect.service", Some(&collect.service)))
                }
                SecretSpec::Generate(_) => None,
            });
        let bot_services = self.manifest.bots.iter().flat_map(|b| &b.services);
        let bots = bot_services
            .clone()
            .map(|service| ("bots.services", Some(service)));
        let gpu = self
            .manifest
            .gpu
            .iter()
            .flat_map(|g| &g.services)
            .map(|service| ("gpu.services", Some(service)));
        let lan = self
            .manifest
            .lan
            .iter()
            .flat_map(|l| &l.ports)
            .map(|port| ("lan.ports", Some(&port.service)));
        let all = refs.iter().cloned().chain(collects).chain(bots).chain(gpu);
        for (field, service) in all.chain(lan) {
            if let Some(service) = service
                && !self.services.contains_key(service)
            {
                return Err(PackageError::UnknownService {
                    field,
                    service: service.clone(),
                });
            }
        }
        // 全插件一份的入口不能指向每个 Bot 一份的服务。
        let bot_entry = refs.iter().find_map(|(field, service)| {
            service
                .filter(|s| bot_services.clone().any(|b| b == *s))
                .map(|s| (*field, s))
        });
        if let Some((field, service)) = bot_entry {
            return Err(PackageError::Service {
                service: service.clone(),
                message: format!("{field} 不能指向 Bot 服务：Bot 服务每个 Bot 一份"),
            });
        }
        if let Some(provider) = egress
            && self.services[&provider.service]
                .as_object()
                .is_some_and(|service| service.contains_key("network_mode"))
        {
            return Err(PackageError::Service {
                service: provider.service.clone(),
                message: "provides.egress_proxy.service 不能写 network_mode".to_owned(),
            });
        }
        Ok(())
    }
}

/// serde 报错带字段路径（如 `webui.port`）：路径在根上时没有可写的段，原样返回信息。
fn pathed<E: fmt::Display>(error: &serde_path_to_error::Error<E>) -> String {
    let path = error.path().to_string();
    if path.is_empty() {
        error.inner().to_string()
    } else {
        format!("`{path}`：{}", error.inner())
    }
}

/// 从 YAML 文本直接反序列化清单：`serde_json::from_value` 丢掉了位置，失败时用它把报错写到
/// 字段和行号（如 `x-wonton.description.Y`，加 serde-saphyr 给的 at line N, column M）。
fn manifest_from_yaml(source: &str) -> Result<Manifest, ManifestError> {
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

fn check_top_level_keys(top: &Map<String, Value>) -> Result<(), PackageError> {
    for key in top.keys() {
        if let Some((_, reason)) = FORBIDDEN_TOP_KEYS.iter().find(|(k, _)| k == key) {
            return Err(PackageError::TopLevel {
                key: key.clone(),
                reason: (*reason).to_owned(),
            });
        }
        let known = matches!(
            key.as_str(),
            "name" | "version" | "services" | "volumes" | "configs"
        );
        if !known && !key.starts_with("x-") {
            return Err(PackageError::TopLevel {
                key: key.clone(),
                reason: "不在核心支持的 Compose 子集里".to_owned(),
            });
        }
    }
    Ok(())
}

/// 服务名同时用作网络别名和容器名的一部分，限制为小写 DNS 安全字符。
fn check_service_name(name: &str) -> Result<(), PackageError> {
    if matches!(name, "relay-webui" | "relay-mcp") {
        return Err(PackageError::Service {
            service: name.to_owned(),
            message: "核心中继的保留服务名，插件不能使用".to_owned(),
        });
    }
    let valid = !name.is_empty()
        && name.len() <= 63
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if !valid {
        return Err(PackageError::Service {
            service: name.to_owned(),
            message: "服务名只能包含小写字母、数字、- 和 _".to_owned(),
        });
    }
    // 容器名是 `<插件网络>--<服务名>`，核心在容器里时按它访问插件。带 `--` 的网络别名可以和别的插件的容器名一样，抢走发给它的
    // 请求。Bot 服务的别名是 `<服务名>-default`，所以结尾也不能是 `-`。
    if name.ends_with('-') || name.contains("--") {
        return Err(PackageError::Service {
            service: name.to_owned(),
            message: "服务名不能以 - 结尾，也不能含 --".to_owned(),
        });
    }
    Ok(())
}

fn check_service_keys(name: &str, service: &Value) -> Result<(), PackageError> {
    let service_err = |message: String| PackageError::Service {
        service: name.to_owned(),
        message,
    };
    let Value::Object(map) = service else {
        return Err(service_err("服务定义必须是映射".to_owned()));
    };
    for key in map.keys() {
        if let Some((_, reason)) = FORBIDDEN_SERVICE_KEYS.iter().find(|(k, _)| k == key) {
            return Err(service_err(format!("不支持 `{key}`：{reason}")));
        }
    }
    match map.get("network_mode") {
        None => Ok(()),
        Some(Value::String(mode)) if matches!(mode.as_str(), "none" | "host") => Ok(()),
        Some(other) => {
            let mode = other
                .as_str()
                .map_or_else(|| other.to_string(), str::to_owned);
            Err(service_err(format!(
                "不支持 network_mode `{mode}`：{}",
                network_mode_refusal(&mode)
            )))
        }
    }
}

fn parse_named_volumes(value: Option<Value>) -> Result<BTreeSet<String>, PackageError> {
    let Some(value) = value else {
        return Ok(BTreeSet::new());
    };
    let Value::Object(map) = value else {
        return Err(PackageError::Volume("volumes".to_owned()));
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
                Err(PackageError::Volume(name))
            }
        })
        .collect()
}

/// 顶层 `configs`：每项恰好有 `file`（插件包里的文件）或 `content`（内联内容）之一。
fn parse_configs(value: Option<Value>) -> Result<BTreeMap<String, ConfigSource>, PackageError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let Value::Object(map) = value else {
        return Err(PackageError::TopLevel {
            key: "configs".to_owned(),
            reason: "必须是映射".to_owned(),
        });
    };
    map.into_iter()
        .map(|(name, mut definition)| {
            let config_err = |message: String| PackageError::Config {
                name: name.clone(),
                message,
            };
            let name_ok = name.starts_with(|c: char| c.is_ascii_alphanumeric())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
            if !name_ok {
                return Err(config_err("配置名只能包含字母、数字、-、_ 和 .".to_owned()));
            }
            strip_extensions(&mut definition);
            let Value::Object(fields) = definition else {
                return Err(config_err("必须是映射，写明 file 或 content".to_owned()));
            };
            for key in fields.keys() {
                if let Some((_, reason)) = FORBIDDEN_CONFIG_KEYS.iter().find(|(k, _)| k == key) {
                    return Err(config_err(format!("不支持 `{key}`：{reason}")));
                }
                if !matches!(key.as_str(), "file" | "content") {
                    return Err(config_err(format!("不支持 `{key}`")));
                }
            }
            let source = match (fields.get("file"), fields.get("content")) {
                (Some(Value::String(path)), None) => {
                    ConfigSource::File(package_files::normalize(path).map_err(config_err)?)
                }
                (None, Some(Value::String(text))) => ConfigSource::Content(text.clone()),
                (Some(_), Some(_)) => return Err(config_err("file 和 content 只能写一个".into())),
                (None, None) => return Err(config_err("必须写 file 或 content".to_owned())),
                _ => return Err(config_err("file 和 content 必须是字符串".to_owned())),
            };
            Ok((name, source))
        })
        .collect()
}
