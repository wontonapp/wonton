//! 插件包：一个带 `x-wonton` 扩展字段的 compose 文件。
//!
//! 解析分两步：[`PluginPackage::parse`] 只看结构和清单，不依赖安装环境；
//! [`PluginPackage::services`] 在给定变量表后做插值，再按 Compose 子集解析每个服务。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::compose::{FORBIDDEN_SERVICE_KEYS, FORBIDDEN_TOP_KEYS, Service, strip_extensions};
use super::interpolate::{InterpolateError, Vars, interpolate_value};
use super::manifest::{Manifest, ManifestError};

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
}

/// 解析好的插件包。
#[derive(Debug, Clone)]
pub struct PluginPackage {
    pub manifest: Manifest,
    /// 原始 compose 文本，安装后原样保存，便于审计和重装。
    pub source: String,
    services: Map<String, Value>,
    volumes: BTreeSet<String>,
}

impl PluginPackage {
    pub fn parse(source: &str) -> Result<Self, PackageError> {
        let document: Value =
            serde_saphyr::from_str(source).map_err(|e| PackageError::Yaml(e.to_string()))?;
        let Value::Object(mut top) = document else {
            return Err(PackageError::NotAMapping);
        };
        check_top_level_keys(&top)?;
        let manifest_value = top
            .remove("x-wonton")
            .ok_or(PackageError::MissingManifest)?;
        let manifest: Manifest = serde_json::from_value(manifest_value)
            .map_err(|e| ManifestError::Invalid(e.to_string()))?;
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
        let package = Self {
            manifest,
            source: source.to_owned(),
            services,
            volumes,
        };
        package.check_manifest_references()?;
        Ok(package)
    }

    pub fn service_names(&self) -> impl Iterator<Item = &str> {
        self.services.keys().map(String::as_str)
    }

    pub fn named_volumes(&self) -> &BTreeSet<String> {
        &self.volumes
    }

    /// 用变量表插值后按 Compose 子集解析所有服务。
    pub fn services(&self, vars: &Vars) -> Result<BTreeMap<String, Service>, PackageError> {
        self.services
            .iter()
            .map(|(name, raw)| {
                let mut value = raw.clone();
                strip_extensions(&mut value);
                interpolate_value(&mut value, vars).map_err(|source| {
                    PackageError::Interpolate {
                        service: name.clone(),
                        source,
                    }
                })?;
                let service: Service =
                    serde_json::from_value(value).map_err(|e| PackageError::Service {
                        service: name.clone(),
                        message: e.to_string(),
                    })?;
                Ok((name.clone(), service))
            })
            .collect()
    }

    fn check_manifest_references(&self) -> Result<(), PackageError> {
        let refs = [
            (
                "webui.service",
                self.manifest.webui.as_ref().map(|w| &w.service),
            ),
            (
                "mcp.service",
                self.manifest.mcp.as_ref().map(|m| &m.service),
            ),
        ];
        for (field, service) in refs {
            if let Some(service) = service
                && !self.services.contains_key(service)
            {
                return Err(PackageError::UnknownService {
                    field,
                    service: service.clone(),
                });
            }
        }
        Ok(())
    }
}

fn check_top_level_keys(top: &Map<String, Value>) -> Result<(), PackageError> {
    for key in top.keys() {
        if let Some((_, reason)) = FORBIDDEN_TOP_KEYS.iter().find(|(k, _)| k == key) {
            return Err(PackageError::TopLevel {
                key: key.clone(),
                reason: (*reason).to_owned(),
            });
        }
        let known = matches!(key.as_str(), "name" | "version" | "services" | "volumes");
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
    let valid = !name.is_empty()
        && name.len() <= 63
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(PackageError::Service {
            service: name.to_owned(),
            message: "服务名只能包含小写字母、数字、- 和 _".to_owned(),
        })
    }
}

fn check_service_keys(name: &str, service: &Value) -> Result<(), PackageError> {
    let Value::Object(map) = service else {
        return Err(PackageError::Service {
            service: name.to_owned(),
            message: "服务定义必须是映射".to_owned(),
        });
    };
    for key in map.keys() {
        if let Some((_, reason)) = FORBIDDEN_SERVICE_KEYS.iter().find(|(k, _)| k == key) {
            return Err(PackageError::Service {
                service: name.to_owned(),
                message: format!("不支持 `{key}`：{reason}"),
            });
        }
    }
    Ok(())
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
