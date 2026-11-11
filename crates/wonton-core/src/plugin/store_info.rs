//! 清单里的商店信息（插件规范的“商店信息”）：规范版本 `spec`、分类、作者，以及各项的格式和上限。

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::languages::Languages;
use super::manifest::{LocalizedText, Manifest, ManifestError, require_locales};
use super::package_files;
use crate::l10n;

/// 核心支持的最新规范版本。
pub const SPEC_VERSION: SpecVersion = SpecVersion(0, 12);

/// 规范版本 `主.次`，如 `0.2`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpecVersion(u32, u32);

impl SpecVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self(major, minor)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (major, minor) = text.split_once('.')?;
        let number = |part: &str| {
            let plain = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'));
            plain.then(|| part.parse().ok()).flatten()
        };
        Some(Self(number(major)?, number(minor)?))
    }
}

impl fmt::Display for SpecVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0, self.1)
    }
}

/// 插件包的作者。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

const MAX_SCREENSHOTS: usize = 6;
const MAX_TAGS: usize = 8;
const MAX_TAG_CHARS: usize = 24;
const MAX_AUTHOR_CHARS: usize = 64;
const MAX_ABOUT_CHARS: usize = 4000;
const MAX_RELEASE_NOTES_CHARS: usize = 2000;
const MAX_SETUP_CHARS: usize = 400;

/// 解析清单之前先看 `spec`：比核心新的清单可能用了核心不认识的字段，这时先报规范版本，不逐项报未知字段。
pub fn check_spec(manifest: &Value, langs: &Languages) -> Result<(), ManifestError> {
    match manifest.get("spec") {
        None => Err(ManifestError::Invalid(l10n::text(
            langs,
            "plugin-settings-store-info-spec-missing",
            &[("version", SPEC_VERSION.to_string().into())],
        ))),
        Some(Value::String(text)) => spec_version(text, langs).map(|_| ()),
        Some(_) => Err(ManifestError::Invalid(l10n::text(
            langs,
            "plugin-settings-store-info-spec-string",
            &[("version", SPEC_VERSION.to_string().into())],
        ))),
    }
}

fn spec_version(text: &str, langs: &Languages) -> Result<SpecVersion, ManifestError> {
    let version = SpecVersion::parse(text).ok_or_else(|| {
        ManifestError::Invalid(l10n::text(
            langs,
            "plugin-settings-store-info-spec-version",
            &[
                ("text", text.into()),
                ("version", SPEC_VERSION.to_string().into()),
            ],
        ))
    })?;
    if version > SPEC_VERSION {
        return Err(ManifestError::NewerSpec {
            spec: text.to_owned(),
            supported: SPEC_VERSION.to_string(),
        });
    }
    Ok(version)
}

/// 校验商店信息。字段的格式见插件规范的“商店信息”；图标和截图的内容在安装和 `plugin check` 时由
/// [`super::images`] 检查。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    spec_version(&manifest.spec, langs)?;
    if let Some(screenshots) = &manifest.screenshots {
        validate_screenshots(screenshots, langs)?;
    }
    if let Some(author) = &manifest.author {
        let name = author.name.trim();
        if name.is_empty() || author.name.chars().count() > MAX_AUTHOR_CHARS {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-store-info-author",
                &[("max", MAX_AUTHOR_CHARS.to_string().into())],
            )));
        }
        if let Some(url) = &author.url {
            https_url("author.url", url, langs)?;
        }
    }
    if let Some(homepage) = &manifest.homepage {
        https_url("homepage", homepage, langs)?;
    }
    if let Some(license) = &manifest.license {
        spdx::Expression::parse(license).map_err(|e| {
            invalid(l10n::text(
                langs,
                "plugin-settings-store-info-license",
                &[
                    ("license", license.as_str().into()),
                    ("error", e.reason.to_string().into()),
                ],
            ))
        })?;
    }
    for tags in manifest.tags.values() {
        validate_tags(tags, langs)?;
    }
    if let Some(about) = &manifest.about {
        long_text("about", about, MAX_ABOUT_CHARS, langs)?;
    }
    if let Some(notes) = &manifest.release_notes {
        long_text("release_notes", notes, MAX_RELEASE_NOTES_CHARS, langs)?;
    }
    if let Some(setup) = &manifest.setup {
        long_text("setup", setup, MAX_SETUP_CHARS, langs)?;
    }
    Ok(())
}

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

fn validate_screenshots(screenshots: &[String], langs: &Languages) -> Result<(), ManifestError> {
    if screenshots.is_empty() || screenshots.len() > MAX_SCREENSHOTS {
        return Err(invalid(l10n::text(
            langs,
            "plugin-settings-store-info-screenshots-count",
            &[("max", MAX_SCREENSHOTS.to_string().into())],
        )));
    }
    let mut seen = BTreeSet::new();
    for raw in screenshots {
        let path =
            package_files::normalize(raw).map_err(|e| invalid(format!("screenshots：{e}")))?;
        let lower = path.to_ascii_lowercase();
        if ![".png", ".jpg", ".jpeg"]
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-store-info-screenshot-format",
                &[("raw", raw.as_str().into())],
            )));
        }
        if !seen.insert(path) {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-store-info-screenshot-duplicate",
                &[("raw", raw.as_str().into())],
            )));
        }
    }
    Ok(())
}

fn validate_tags(tags: &[String], langs: &Languages) -> Result<(), ManifestError> {
    if tags.len() > MAX_TAGS {
        return Err(invalid(l10n::text(
            langs,
            "plugin-settings-store-info-tags-count",
            &[("max", MAX_TAGS.to_string().into())],
        )));
    }
    let mut seen = BTreeSet::new();
    for tag in tags {
        let chars = tag.chars().count();
        if tag.trim() != tag || !(1..=MAX_TAG_CHARS).contains(&chars) {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-store-info-tag-length",
                &[
                    ("tag", tag.as_str().into()),
                    ("max", MAX_TAG_CHARS.to_string().into()),
                ],
            )));
        }
        if tag.chars().any(char::is_control) {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-store-info-tag-control",
                &[("tag", tag.as_str().into())],
            )));
        }
        if !seen.insert(tag.to_lowercase()) {
            return Err(invalid(l10n::text(
                langs,
                "plugin-settings-store-info-tag-duplicate",
                &[("tag", tag.as_str().into())],
            )));
        }
    }
    Ok(())
}

fn long_text(
    field: &'static str,
    text: &LocalizedText,
    max_chars: usize,
    langs: &Languages,
) -> Result<(), ManifestError> {
    require_locales(field, text)?;
    match text
        .iter()
        .find(|(_, value)| value.chars().count() > max_chars)
    {
        Some((lang, _)) => Err(invalid(l10n::text(
            langs,
            "plugin-settings-store-info-field-length",
            &[
                ("field", field.into()),
                ("lang", lang.as_str().into()),
                ("max", max_chars.to_string().into()),
            ],
        ))),
        None => Ok(()),
    }
}

fn https_url(field: &str, value: &str, langs: &Languages) -> Result<(), ManifestError> {
    match url::Url::parse(value) {
        Ok(url) if url.scheme() == "https" && url.host_str().is_some() => Ok(()),
        _ => Err(invalid(l10n::text(
            langs,
            "plugin-settings-store-info-https-url",
            &[("field", field.into()), ("value", value.into())],
        ))),
    }
}

#[cfg(test)]
mod tests;
