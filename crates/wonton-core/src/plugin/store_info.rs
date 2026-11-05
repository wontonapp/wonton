//! 清单里的商店信息（插件规范的“商店信息”）：规范版本 `spec`、分类、作者，以及各项的格式和上限。

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::manifest::{LocalizedText, Manifest, ManifestError, require_locales};
use super::package_files;

/// 核心支持的最新规范版本。
pub const SPEC_VERSION: SpecVersion = SpecVersion(0, 6);

/// 规范版本 `主.次`，如 `0.2`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpecVersion(u32, u32);

impl SpecVersion {
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

/// 解析清单之前先看 `spec`：比核心新的清单可能用了核心不认识的字段，这时先报规范版本，不逐项报未知字段。
pub fn check_spec(manifest: &Value) -> Result<(), ManifestError> {
    match manifest.get("spec") {
        None => Err(ManifestError::Invalid(format!(
            "清单要写 spec，如 spec: \"{SPEC_VERSION}\""
        ))),
        Some(Value::String(text)) => spec_version(text).map(|_| ()),
        Some(_) => Err(ManifestError::Invalid(format!(
            "spec 必须是字符串，如 \"{SPEC_VERSION}\"（YAML 里要加引号，不然读成数字）"
        ))),
    }
}

fn spec_version(text: &str) -> Result<SpecVersion, ManifestError> {
    let version = SpecVersion::parse(text).ok_or_else(|| {
        ManifestError::Invalid(format!(
            "spec `{text}` 不是规范版本（如 \"{SPEC_VERSION}\"）"
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
pub fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    spec_version(&manifest.spec)?;
    if let Some(screenshots) = &manifest.screenshots {
        validate_screenshots(screenshots)?;
    }
    if let Some(author) = &manifest.author {
        let name = author.name.trim();
        if name.is_empty() || author.name.chars().count() > MAX_AUTHOR_CHARS {
            return Err(invalid(format!(
                "author.name 不能为空，不超过 {MAX_AUTHOR_CHARS} 个字符"
            )));
        }
        if let Some(url) = &author.url {
            https_url("author.url", url)?;
        }
    }
    if let Some(homepage) = &manifest.homepage {
        https_url("homepage", homepage)?;
    }
    if let Some(license) = &manifest.license {
        spdx::Expression::parse(license).map_err(|e| {
            invalid(format!(
                "license `{license}` 不是 SPDX 许可证表达式（如 MIT、AGPL-3.0-only）：{}",
                e.reason
            ))
        })?;
    }
    for tags in manifest.tags.values() {
        validate_tags(tags)?;
    }
    if let Some(about) = &manifest.about {
        long_text("about", about, MAX_ABOUT_CHARS)?;
    }
    if let Some(notes) = &manifest.release_notes {
        long_text("release_notes", notes, MAX_RELEASE_NOTES_CHARS)?;
    }
    Ok(())
}

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

fn validate_screenshots(screenshots: &[String]) -> Result<(), ManifestError> {
    if screenshots.is_empty() || screenshots.len() > MAX_SCREENSHOTS {
        return Err(invalid(format!(
            "screenshots 要有 1 到 {MAX_SCREENSHOTS} 张"
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
            return Err(invalid(format!("截图 `{raw}` 要是 PNG 或 JPEG")));
        }
        if !seen.insert(path) {
            return Err(invalid(format!("截图 `{raw}` 重复了")));
        }
    }
    Ok(())
}

fn validate_tags(tags: &[String]) -> Result<(), ManifestError> {
    if tags.len() > MAX_TAGS {
        return Err(invalid(format!("tags 最多 {MAX_TAGS} 个")));
    }
    let mut seen = BTreeSet::new();
    for tag in tags {
        let chars = tag.chars().count();
        if tag.trim() != tag || !(1..=MAX_TAG_CHARS).contains(&chars) {
            return Err(invalid(format!(
                "标签 `{tag}` 要有 1 到 {MAX_TAG_CHARS} 个字符，前后不带空白"
            )));
        }
        if tag.chars().any(char::is_control) {
            return Err(invalid(format!("标签 `{tag}` 里有控制字符")));
        }
        if !seen.insert(tag.to_lowercase()) {
            return Err(invalid(format!("标签 `{tag}` 重复了")));
        }
    }
    Ok(())
}

fn long_text(
    field: &'static str,
    text: &LocalizedText,
    max_chars: usize,
) -> Result<(), ManifestError> {
    require_locales(field, text)?;
    match text
        .iter()
        .find(|(_, value)| value.chars().count() > max_chars)
    {
        Some((lang, _)) => Err(invalid(format!("{field}.{lang} 超过 {max_chars} 个字符"))),
        None => Ok(()),
    }
}

fn https_url(field: &str, value: &str) -> Result<(), ManifestError> {
    match url::Url::parse(value) {
        Ok(url) if url.scheme() == "https" && url.host_str().is_some() => Ok(()),
        _ => Err(invalid(format!(
            "{field} `{value}` 要是 https:// 开头的网址"
        ))),
    }
}

#[cfg(test)]
mod tests;
