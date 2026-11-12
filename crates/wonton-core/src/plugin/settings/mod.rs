//! 插件设置（ADR-0077）：包里的 Schema、校验过的值、表单和注入变量。清单的 `x-wonton.settings` 指到这个文件。
//!
//! 值存进插件记录、明文可读；敏感项留在输入密钥（ADR-0064）。引擎侧的读写和重建在
//! [`super::settings_update`]，HTTP 路由在网关。

pub(crate) mod schema;
pub(crate) mod value;

use std::collections::BTreeMap;

use serde_json::Value;

use super::install_error::InstallError;
use super::languages::Languages;
use super::manifest::Manifest;
use super::package::PluginPackage;
use crate::l10n;
pub(crate) use schema::Schema;
pub(crate) use value::Values;
use wonton_protocol::{PluginSettingsView, SettingsForm, SettingsSchemaProblem};

/// 设置值的 Compose 插值变量名。
pub(super) fn setting_var(name: &str) -> String {
    format!("WONTON_SETTING_{}", name.to_ascii_uppercase())
}

/// 这个版本的清单有没有设置。服务端支持设置接口时才置详情里的 `has_settings`。
pub(crate) fn enabled(manifest: &Manifest) -> bool {
    manifest.settings.is_some()
}

/// 读并校验插件包里的 Schema。没声明时为空；不合格时把问题都带回去。
pub(crate) fn load(
    package: &PluginPackage,
    langs: &Languages,
) -> Result<Option<Schema>, InstallError> {
    let Some(bytes) = package.settings() else {
        return Ok(None);
    };
    match schema::parse(&package.manifest, bytes, langs) {
        Ok(schema) => Ok(Some(schema)),
        Err(problems) => Err(schema_error(problems, langs)),
    }
}

pub(crate) fn schema_error(
    problems: Vec<SettingsSchemaProblem>,
    langs: &Languages,
) -> InstallError {
    let count = problems.len();
    InstallError::SettingsSchemaInvalid(
        l10n::text(
            langs,
            "plugin-settings-schema-problems",
            &[("count", (count as i64).into())],
        ),
        problems,
    )
}

pub(crate) fn value_error(
    fields: Vec<wonton_protocol::SettingsFieldError>,
    langs: &Languages,
) -> InstallError {
    let count = fields.len();
    InstallError::SettingsInvalid(
        l10n::text(
            langs,
            "plugin-settings-value-problems",
            &[("count", (count as i64).into())],
        ),
        fields,
    )
}

/// 新装时要存的设置值：显式给的值叠上 Schema 默认值再校验。插件没有设置、却给了值时，每项都报不认识。
pub(crate) fn install_values(
    package: &PluginPackage,
    given: &wonton_protocol::SettingValues,
    langs: &Languages,
) -> Result<Values, InstallError> {
    match load(package, langs)? {
        Some(schema) => {
            value::initial(&schema, given, langs).map_err(|fields| value_error(fields, langs))
        }
        None if given.is_empty() => Ok(Values::new()),
        None => Err(value_error(
            given
                .keys()
                .map(|name| wonton_protocol::SettingsFieldError {
                    name: name.clone(),
                    kind: wonton_protocol::SettingsFieldProblemKind::UnknownField,
                    message: l10n::text(langs, "plugin-settings-value-unknown-field", &[]),
                })
                .collect(),
            langs,
        )),
    }
}

/// 读设置页的视图；`services` 是每个设置被哪些服务引用。
pub(crate) fn view(
    schema: &Schema,
    values: &Values,
    services: &BTreeMap<String, Vec<String>>,
    changed_at_ms: Option<u64>,
    langs: &Languages,
) -> PluginSettingsView {
    let mut form = value::form(schema, values, langs);
    for field in &mut form.fields {
        field.services = services.get(&field.name).cloned().unwrap_or_default();
    }
    PluginSettingsView {
        schema_digest: schema.digest.clone(),
        form,
        changed_at_ms,
    }
}

/// 安装、升级预览里的表单：默认值已经填好，服务列为空（还没装）。
pub(crate) fn install_form(schema: &Schema, values: &Values, langs: &Languages) -> SettingsForm {
    value::form(schema, values, langs)
}

/// 从渲染结果里数出每个设置被哪些服务引用。
pub(crate) fn services(
    schema: &Schema,
    containers: &[super::render::ContainerPlan],
) -> BTreeMap<String, Vec<String>> {
    schema
        .fields
        .iter()
        .map(|field| {
            let variable = setting_var(&field.name);
            let users = containers
                .iter()
                .filter(|container| container.variables.contains(&variable))
                .map(|container| container.service.clone())
                .collect();
            (field.name.clone(), users)
        })
        .collect()
}

/// dry run 的默认值：Schema 坏了也照渲染（问题由预览单独报）。
pub(crate) fn defaults(package: &PluginPackage) -> BTreeMap<String, Value> {
    let Some(bytes) = package.settings() else {
        return BTreeMap::new();
    };
    let langs = Languages::default();
    let Ok(schema) = schema::parse(&package.manifest, bytes, &langs) else {
        return BTreeMap::new();
    };
    schema
        .fields
        .iter()
        .filter_map(|field| {
            field
                .default
                .clone()
                .map(|value| (field.name.clone(), value))
        })
        .collect()
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod value_tests;
