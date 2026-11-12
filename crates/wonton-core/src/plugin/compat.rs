//! 插件兼容裁决（ADR-0085）：这份核心读不读得懂一份插件包。目标镜像用
//! `wonton-core plugin compat <插件包目录>…` 自证自己的支持范围，JSON 写标准输出；
//! 当前核心升级前在一次性容器里对目标镜像跑它，解析结果做"只报新失效"的对照。
//!
//! 检查范围和安装同一套：解析清单（含未知字段拒绝）、`Manifest::validate`、[`dry_run`]
//! 渲染；不含 `plugin check` 的打包质量和主机要求。类别只有四个，`raw` 是清单原值，
//! JSON 里没有人话。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::capabilities;
use super::languages::Languages;
use super::manifest::{ManifestError, Permission};
use super::package::{PackageError, PluginPackage};
use super::package_files::COMPOSE_FILE;
use super::render::{RenderError, dry_run};
use super::store_info::SPEC_VERSION;

/// 原因类别的线上字符串：消费方逐字对照，契约测试钉住。
pub const SPEC_UNSUPPORTED: &str = "spec_unsupported";
pub const UNKNOWN_FEATURE: &str = "unknown_feature";
pub const CAPABILITY_MISSING: &str = "capability_missing";
pub const OTHER: &str = "other";

/// compat 的标准输出。`spec_supported` 和 `capabilities` 说明这份核心自己支持到哪。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub spec_supported: String,
    pub capabilities: Vec<String>,
    pub plugins: Vec<PackageVerdict>,
}

/// 一份插件包的裁决。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageVerdict {
    pub id: String,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Breaking,
}

/// 一项读不懂的原因：类别加清单原值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub category: String,
    pub raw: String,
}

impl Reason {
    fn new(category: &str, raw: impl Into<String>) -> Self {
        Self {
            category: category.to_owned(),
            raw: raw.into(),
        }
    }
}

impl PackageVerdict {
    fn ok(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            verdict: Verdict::Ok,
            reasons: Vec::new(),
        }
    }

    fn breaking(id: impl Into<String>, reasons: Vec<Reason>) -> Self {
        Self {
            id: id.into(),
            verdict: Verdict::Breaking,
            reasons,
        }
    }
}

/// 对每个插件包目录裁决一遍。子命令入口：一个坏包不挡别的包。
pub fn output(dirs: &[&Path], langs: &Languages) -> Output {
    Output {
        spec_supported: SPEC_VERSION.to_string(),
        capabilities: capabilities::ALL
            .iter()
            .map(|capability| capability.name.to_owned())
            .collect(),
        plugins: dirs.iter().map(|dir| judge_dir(dir, langs)).collect(),
    }
}

/// 从插件包目录裁决：解析和安装读同一套文件。
pub fn judge_dir(dir: &Path, langs: &Languages) -> PackageVerdict {
    let fallback = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(source) = std::fs::read_to_string(dir.join(COMPOSE_FILE)) else {
        return PackageVerdict::breaking(fallback, vec![Reason::new(OTHER, "")]);
    };
    judge(&source, langs, &fallback, |langs| {
        PluginPackage::load_dir(dir, langs)
    })
}

/// 从 compose 原文裁决，真实解析交给 `load`：安装路径是
/// [`PluginPackage::with_files`]（官方索引的条目没有目录，被引用的文件用占位内容），
/// 插件包目录用 [`PluginPackage::load_dir`]。
pub fn judge(
    source: &str,
    langs: &Languages,
    id_fallback: &str,
    load: impl Fn(&Languages) -> Result<super::package::PluginPackage, PackageError>,
) -> PackageVerdict {
    let Some(manifest) = manifest_value(source) else {
        return PackageVerdict::breaking(id_fallback, vec![Reason::new(OTHER, "")]);
    };
    let id = manifest
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(id_fallback)
        .to_owned();
    // 和安装同一裁决：spec 在字段解析之前，比核心新的清单先报规范版本，不逐项报未知字段。
    match super::store_info::check_spec(&manifest, langs) {
        Err(ManifestError::NewerSpec { spec, .. }) => {
            return PackageVerdict::breaking(id, vec![Reason::new(SPEC_UNSUPPORTED, spec)]);
        }
        Err(_) => {
            let raw = manifest.get("spec").and_then(Value::as_str).unwrap_or("");
            return PackageVerdict::breaking(id, vec![Reason::new(OTHER, raw)]);
        }
        Ok(()) => {}
    }
    // 权限表和能力表逐项扫描：每项都拿这份核心自己的表判断，报全所有不认识的名字
    // （安装路径只报第一个，修完才会见到下一个）。
    let scanned = scan(&manifest);
    match load(langs) {
        Ok(package) => match dry_run(&package, langs) {
            Ok(_) => PackageVerdict::ok(package.manifest.id.to_string()),
            Err(error) => {
                PackageVerdict::breaking(id, vec![Reason::new(OTHER, render_raw(&error))])
            }
        },
        Err(error) => {
            if !scanned.is_empty() {
                return PackageVerdict::breaking(id, scanned);
            }
            PackageVerdict::breaking(id, vec![classify(source, &error)])
        }
    }
}

/// 清单的 JSON 值：和安装一样经 serde_saphyr 进 serde_json。
fn manifest_value(source: &str) -> Option<Value> {
    let document: Value = serde_saphyr::from_str(source).ok()?;
    let manifest = document.get("x-wonton")?;
    manifest.is_object().then(|| manifest.clone())
}

fn scan(manifest: &Value) -> Vec<Reason> {
    let mut reasons = Vec::new();
    let permissions = manifest
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let optional = manifest
        .get("optional_permissions")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|optional| optional.keys().map(String::as_str));
    for name in permissions.chain(optional) {
        if name.parse::<Permission>().is_err() {
            reasons.push(Reason::new(UNKNOWN_FEATURE, name));
        }
    }
    let required = ["requires", "wants"]
        .into_iter()
        .filter_map(|field| manifest.get(field))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str);
    let provided = manifest
        .get("provides")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|provides| provides.keys().map(String::as_str));
    for name in required.chain(provided) {
        if capabilities::lookup(name).is_none() && !capabilities::is_third_party(name) {
            reasons.push(Reason::new(CAPABILITY_MISSING, name));
        }
    }
    reasons
}

/// 扫描没抓到的解析失败。未知字段借安装路径同一份清单类型重解析拿字段路径；
/// 其余（引用的文件缺失、compose 超出子集、清单写得不合法等）如实归入 other，
/// raw 给得出原值就给，给不出留空。
fn classify(source: &str, error: &PackageError) -> Reason {
    if let PackageError::Interpolate { source, .. }
    | PackageError::ConfigInterpolate { source, .. } = error
    {
        return Reason::new(OTHER, source.variable().unwrap_or_default());
    }
    if matches!(error, PackageError::Manifest(_))
        && let Some(path) = unknown_field_path(source)
    {
        return Reason::new(UNKNOWN_FEATURE, path);
    }
    Reason::new(OTHER, "")
}

/// 清单解析不了时重解析一遍拿未知字段的路径（如 `x-wonton.tabs`）当原值。只有 serde 的
/// "unknown field"算不认识的字段；标量松紧、写得不合法等没有路径或不算不认识的，都是 `None`。
fn unknown_field_path(source: &str) -> Option<String> {
    let ManifestError::Invalid(text) = super::package::manifest_from_yaml(source).err()? else {
        return None;
    };
    if !text.contains("unknown field") {
        return None;
    }
    // serde_path_to_error 的路径在开头的一对反引号里；路径为空时只有报错本身。
    let rest = text.strip_prefix('`')?;
    let end = rest.find('`')?;
    Some(rest[..end].to_owned())
}

/// 渲染失败时能给出的原值：因为哪个变量没注入而失败时是变量名。
fn render_raw(error: &RenderError) -> String {
    match error {
        RenderError::Package(
            PackageError::Interpolate { source, .. }
            | PackageError::ConfigInterpolate { source, .. },
        ) => source.variable().unwrap_or_default().to_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "compat_tests.rs"]
mod tests;
