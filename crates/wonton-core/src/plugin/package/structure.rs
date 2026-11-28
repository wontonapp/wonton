//! 从 compose 文本解析出插件包的结构：顶层键、服务、命名卷和清单。

use std::collections::BTreeSet;

use serde::Deserialize;
// `serde_saphyr::Error::custom` 是 serde 的 trait 方法，路径经它带出反序列化器闭包。
use serde::de::Error as _;
use serde_json::{Map, Value};

use super::super::compose::{
    NetworkMode, forbidden_service_key_reason, forbidden_top_key_reason, network_mode_refusal,
    strip_extensions,
};
use super::super::languages::Languages;
use super::super::manifest::{Manifest, ManifestError, ManifestExt};
use super::super::manifest_static::StaticWebManifestExt;
use super::configs::parse_configs;
use super::errors::{service_error, top_level_error, yaml_error};
use super::{PackageError, PackageFiles, PluginPackage, pathed};
use super::{runtime, static_files};
use crate::l10n::text;

impl PluginPackage {
    /// compose 文本里的清单，不管它引用的文件：商店列表只要清单。
    pub fn manifest_of(source: &str, langs: &Languages) -> Result<Manifest, PackageError> {
        Ok(Self::structure(source, langs)?.manifest)
    }

    pub(super) fn structure(source: &str, langs: &Languages) -> Result<Self, PackageError> {
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
        super::super::store_info::check_spec(&manifest_value, langs)?;
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
        let configs = parse_configs(top.remove("configs"), langs)?;
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
    if super::super::host_relay::reserved_service_name(manifest, name) {
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
    super::super::network_mode::validate_fields(name, map)?;
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
