//! “关于”里的隐私说明和开源许可。
//!
//! 许可清单是构建时生成的 `licenses.json`：壳从 App 资源里读出来交给 [`licenses`]，核心只做筛选和排序，
//! 免得两端各写一遍解析。许可全文随清单一起来，打开某个许可时不再问第二次。

use std::collections::BTreeMap;

use serde::Deserialize;
use wonton_protocol::{
    CoreError, LicenseComponent, LicenseGroup, LicensesViewModel, PrivacySection, PrivacyViewModel,
};

use crate::l10n;

/// 隐私说明的骨架：标题的消息 id 和它下面的段落，文案在 `locales/<语言>/settings.ftl` 的 `privacy-*`。
///
/// 只写代码里核实过的事（ADR-0003 的推送中继只见密文、ADR-0004 的直连与中继、核心取链接预览）；
/// 改了对应的行为要一起改这里。
const SECTIONS: &[(&str, &[&str])] = &[
    (
        "privacy-what-title",
        &["privacy-what-own", "privacy-what-none"],
    ),
    (
        "privacy-messages-title",
        &["privacy-messages-e2ee", "privacy-messages-history"],
    ),
    (
        "privacy-push-title",
        &["privacy-push-keepalive", "privacy-push-unified"],
    ),
    (
        "privacy-network-title",
        &["privacy-network-lan", "privacy-network-relay"],
    ),
    ("privacy-preview-title", &["privacy-preview-core"]),
    (
        "privacy-plugins-title",
        &["privacy-plugins-oidc", "privacy-plugins-web"],
    ),
    (
        "privacy-device-title",
        &["privacy-device-files", "privacy-device-backup"],
    ),
];

pub(crate) fn privacy() -> PrivacyViewModel {
    PrivacyViewModel {
        sections: SECTIONS
            .iter()
            .map(|(title, paragraphs)| PrivacySection {
                title: l10n::text(title, &[]),
                paragraphs: paragraphs.iter().map(|id| l10n::text(id, &[])).collect(),
            })
            .collect(),
    }
}

#[derive(Deserialize)]
struct RawLicenses {
    components: Vec<RawComponent>,
    licenses: BTreeMap<String, RawLicense>,
}

#[derive(Deserialize)]
struct RawComponent {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    license_ids: Vec<String>,
}

#[derive(Deserialize)]
struct RawLicense {
    name: String,
    #[serde(default)]
    text: String,
}

/// 解析清单：一个许可一组，组里是按名字排序的组件。清单里没有组件用到的许可不出现。
pub(crate) fn licenses(json: &str) -> Result<LicensesViewModel, CoreError> {
    let raw: RawLicenses = serde_json::from_str(json).map_err(|error| {
        invalid(l10n::text(
            "licenses-parse-failed",
            &[("error", error.to_string().into())],
        ))
    })?;
    if raw.components.is_empty() {
        return Err(invalid(l10n::text("licenses-empty", &[])));
    }
    let mut licenses: Vec<LicenseGroup> = Vec::new();
    for (id, license) in &raw.licenses {
        let mut components: Vec<LicenseComponent> = raw
            .components
            .iter()
            .filter(|component| component.license_ids.iter().any(|used| used == id))
            .map(|component| LicenseComponent {
                name: component.name.clone(),
                version: component.version.clone(),
            })
            .collect();
        if components.is_empty() {
            continue;
        }
        if license.text.trim().is_empty() {
            return Err(invalid(l10n::text(
                "licenses-no-text",
                &[("id", id.into())],
            )));
        }
        components.sort_by_key(|component| component.name.to_lowercase());
        licenses.push(LicenseGroup {
            id: id.clone(),
            name: license.name.clone(),
            components,
            text: license.text.clone(),
        });
    }
    if licenses.is_empty() {
        return Err(invalid(l10n::text("licenses-none-to-show", &[])));
    }
    licenses.sort_by_key(|license| license.name.to_lowercase());
    Ok(LicensesViewModel { licenses })
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidInput { reason }
}

#[cfg(test)]
#[path = "about_tests.rs"]
mod about_tests;
