//! 插件包的顶层 `configs`：声明的配置、写进容器的内容，以及解析时的校验。

use std::collections::BTreeMap;

use serde_json::Value;

use super::super::compose::{forbidden_config_key_reason, strip_extensions};
use super::super::languages::Languages;
use super::super::package_files;
use super::errors::{config_error, config_interpolate_error, top_level_error};
use super::{PackageError, PluginPackage};
use crate::l10n::text;
use crate::plugin::interpolate::{UsedVars, Vars, interpolate};

/// 顶层 `configs` 的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// 插件包里的文件（规范化后的相对路径），原样写进容器。
    File(String),
    /// 内联内容，安装时按注入的变量插值。
    Content(String),
}

impl PluginPackage {
    pub fn has_config(&self, name: &str) -> bool {
        self.configs.contains_key(name)
    }

    /// 配置写进容器的内容：`file` 是包里的文件原样，`content` 按变量表插值。
    pub fn config_data(
        &self,
        name: &str,
        vars: &Vars,
        used: &mut UsedVars,
        langs: &Languages,
    ) -> Result<Vec<u8>, PackageError> {
        match self.configs.get(name) {
            None => Err(config_error(
                langs,
                name,
                text(langs, "plugin-install-pkg-config-undeclared", &[]),
            )),
            Some(ConfigSource::File(path)) => self.files.get(path).cloned().ok_or_else(|| {
                config_error(
                    langs,
                    name,
                    text(
                        langs,
                        "plugin-install-pkg-config-missing",
                        &[("path", path.as_str().into())],
                    ),
                )
            }),
            Some(ConfigSource::Content(text)) => interpolate(text, vars, used)
                .map(String::into_bytes)
                .map_err(|source| config_interpolate_error(langs, name.to_owned(), source)),
        }
    }
}

/// 顶层 `configs`：每项恰好有 `file`（插件包里的文件）或 `content`（内联内容）之一。
pub(super) fn parse_configs(
    value: Option<Value>,
    langs: &Languages,
) -> Result<BTreeMap<String, ConfigSource>, PackageError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let Value::Object(map) = value else {
        return Err(top_level_error(
            langs,
            "configs",
            "must be a mapping".to_owned(),
        ));
    };
    map.into_iter()
        .map(|(name, mut definition)| {
            let config_err = |message: String| config_error(langs, &name, message);
            let name_ok = name.starts_with(|c: char| c.is_ascii_alphanumeric())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
            if !name_ok {
                return Err(config_err(
                    "config names allow only letters, digits, -, _, and .".to_owned(),
                ));
            }
            strip_extensions(&mut definition);
            let Value::Object(fields) = definition else {
                return Err(config_err(
                    "must be a mapping with either file or content".to_owned(),
                ));
            };
            for key in fields.keys() {
                if let Some(reason) = forbidden_config_key_reason(key, langs) {
                    return Err(config_err(format!("unsupported key `{key}`: {reason}")));
                }
                if !matches!(key.as_str(), "file" | "content") {
                    return Err(config_err(format!("unsupported key `{key}`")));
                }
            }
            let source = match (fields.get("file"), fields.get("content")) {
                (Some(Value::String(path)), None) => {
                    ConfigSource::File(package_files::normalize(path).map_err(config_err)?)
                }
                (None, Some(Value::String(text))) => ConfigSource::Content(text.clone()),
                (Some(_), Some(_)) => {
                    return Err(config_err("write either file or content, not both".into()));
                }
                (None, None) => return Err(config_err("write either file or content".to_owned())),
                _ => return Err(config_err("file and content must be strings".to_owned())),
            };
            Ok((name, source))
        })
        .collect()
}
