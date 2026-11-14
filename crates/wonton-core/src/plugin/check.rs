//! `wonton-core plugin check <目录> [--official]`（插件规范的“开发与校验”）：和安装时同一套解析和校验，再检查图标和
//! 截图（和核心一样在子进程里处理）；`--official` 再按官方索引的收录要求检查。最后给出插件包摘要。不连 Docker，也不联网。

use std::collections::BTreeSet;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;

use bollard::models::ContainerCreateBody;

use super::core_address::{CIDR_VAR, VAR};
use super::egress::hardcoded_proxy;
use super::image_worker::ImageWorker;
use super::images::IconFormat;
use super::lan::webui::impersonable;
use super::languages::Languages;
use super::manager::supported;
use super::manifest::{Manifest, Permission, PluginKind};
use super::package::PluginPackage;
use super::package_files::{self, COMPOSE_FILE};
use super::render::{Deployment, dry_run};

/// 截图的长边建议至少这么长。
const SCREENSHOT_EDGE: u32 = 1280;

/// 检查的结果。
#[derive(Debug, Default)]
pub struct Report {
    pub id: String,
    pub version: String,
    /// 插件包里的文件和大小（字节），按路径排序。
    pub files: Vec<(String, usize)>,
    pub digest: String,
    /// 不合格的地方：有一项就不该发布。
    pub problems: Vec<String>,
    /// 建议改的地方。
    pub warnings: Vec<String>,
}

/// 检查插件包目录。插件包解析不了时直接报错（和安装时报的一样）；解析得了时把其余的问题都列出来。
pub async fn check(dir: &Path, official: bool, images: &ImageWorker) -> Result<Report, String> {
    let opened = package_files::open_dir(dir).map_err(|e| e.to_string())?;
    check_at(opened.as_fd(), official, images).await
}

/// 同 [`check`]，从已打开的目录读：调用方（如 outbox 的解析）钉住目录，中途换不掉。
pub async fn check_at(
    dir: BorrowedFd<'_>,
    official: bool,
    images: &ImageWorker,
) -> Result<Report, String> {
    let package =
        PluginPackage::load_dir_at(dir, &Languages::default()).map_err(|e| e.to_string())?;
    check_package(&package, dir, official, images).await
}

/// 对已经读出的插件包做检查：装的和校验的是同一份（安装路径先 `load_dir_at` 再调这里），
/// 截图这类不在包里的文件再从目录 fd 读。
pub async fn check_package(
    package: &PluginPackage,
    dir: BorrowedFd<'_>,
    official: bool,
    images: &ImageWorker,
) -> Result<Report, String> {
    let m = &package.manifest;
    let mut files: Vec<(String, usize)> = package
        .files()
        .iter()
        .map(|(path, data)| (path.clone(), data.len()))
        .collect();
    files.push((COMPOSE_FILE.to_owned(), package.source.len()));
    files.sort();
    let mut report = Report {
        id: m.id.to_string(),
        version: m.version.clone(),
        files,
        digest: package.digest(),
        ..Report::default()
    };
    if m.kind == PluginKind::System {
        report
            .warnings
            .push("kind: system plugins can only be preinstalled with the home server; users can't install them"
                .to_owned());
    }
    for permission in m.permissions.iter().filter(|p| !supported(p)) {
        report
            .problems
            .push(format!(
                "permission {permission} isn't supported by this home server yet; update the home server or drop the permission"
            ));
    }
    match dry_run(package, &Languages::default()) {
        Ok(plan) => check_services(&plan, m, official, &mut report),
        Err(e) => report.problems.push(e.to_string()),
    }
    check_spec_floor(package, &mut report);
    check_compose_hygiene(package, &mut report);
    check_trusts_core(package, official, &mut report);
    check_icon(package, official, images, &mut report).await;
    check_tab_icons(package, images, &mut report).await;
    check_tab_title_lengths(package, &mut report);
    super::check_element_call::check(m, &mut report);
    check_screenshots(dir, m, images, &mut report).await;
    if official {
        let required = [
            ("tags", !m.tags.is_empty()),
            ("license", m.license.is_some()),
            ("author", m.author.is_some()),
            ("notices", !m.notices.is_empty()),
        ];
        for (field, _) in required.iter().filter(|(_, present)| !present) {
            report
                .problems
                .push(format!("the official index requires {field}"));
        }
        let keys = |keys: Vec<&String>| ["zh", "en"].iter().all(|l| keys.iter().any(|k| k == l));
        let translated = [
            ("description", keys(m.description.keys().collect())),
            ("tags", m.tags.is_empty() || keys(m.tags.keys().collect())),
            (
                "about",
                m.about.as_ref().is_none_or(|t| keys(t.keys().collect())),
            ),
            (
                "release_notes",
                m.release_notes
                    .as_ref()
                    .is_none_or(|t| keys(t.keys().collect())),
            ),
            (
                "setup",
                m.setup.as_ref().is_none_or(|t| keys(t.keys().collect())),
            ),
        ];
        for (field, _) in translated.iter().filter(|(_, both)| !both) {
            report.problems.push(format!(
                "the official index requires {field} in both zh and en"
            ));
        }
    }
    Ok(report)
}

/// 镜像要按摘要固定，非网页长期服务要有健康检查：官方索引要求，其他插件只提醒。
/// 镜像自带的健康检查看不到（检查不连 Docker）；网页服务可以由核心 HTTP 探测。
fn check_services(plan: &Deployment, manifest: &Manifest, official: bool, report: &mut Report) {
    let webui = manifest.webui.as_ref();
    let list = if official {
        &mut report.problems
    } else {
        &mut report.warnings
    };
    for container in &plan.containers {
        // 中继是核心自己的组件，镜像由核心固定。
        if plan
            .relays
            .iter()
            .any(|r| r.service_name() == container.service)
        {
            continue;
        }
        if !container.image.contains("@sha256:") {
            list.push(format!(
                "service {} image {} isn't pinned by digest (write name:tag@sha256:…)",
                container.service, container.image
            ));
        }
        let probed = webui.is_some_and(|w| w.service == container.service);
        if !container.one_shot && !probed && !has_healthcheck(&container.body) {
            list.push(format!(
                "service {} has no healthcheck; add one, or use webui.health for the web service (its status will show as unknown after install)",
                container.service
            ));
        }
        if official {
            let env = container.body.env.iter().flatten();
            for line in env.filter(|line| hardcoded_proxy(line)) {
                let name = line.split('=').next().unwrap_or_default();
                list.push(format!(
                    "service {} env {name} hardcodes a proxy address: the home server injects proxy variables for egress plugins, and the address won't work on another host (ADR-0023)",
                    container.service
                ));
            }
        }
    }
    // 厂商版镜像和通用镜像一样要按摘要固定（ADR-0099）。
    if official {
        for (service, vendors) in manifest.gpu.iter().flat_map(|gpu| &gpu.images) {
            for (vendor, image) in vendors
                .iter()
                .filter(|(_, image)| !image.contains("@sha256:"))
            {
                list.push(format!(
                    "service {service} image {image} for {} isn't pinned by digest (write name:tag@sha256:…)",
                    vendor.key()
                ));
            }
        }
    }
}

/// 用请求头认用户的应用，信任的代理地址只写核心的地址（ADR-0023 的“只信任核心的地址”）。出网插件能从宿主连到它，所以官方
/// 索引不收；不出网的只有开了局域网端口才有人连得到，那些端口有开关。
/// 用了新版本的字段，`spec` 要跟着写到那一版（[`super::spec_floor`]）。
fn check_spec_floor(package: &PluginPackage, report: &mut Report) {
    match serde_saphyr::from_str::<serde_json::Value>(&package.source) {
        Ok(document) => report
            .problems
            .extend(super::spec_floor::problems(&document)),
        Err(e) => report.problems.push(format!("docker-compose.yml: {e}")),
    }
}

fn check_trusts_core(package: &PluginPackage, official: bool, report: &mut Report) {
    if !impersonable(package) {
        return;
    }
    let m = &package.manifest;
    let advice = format!(
        "the web app authenticates users from request headers but doesn't trust only the home server: set the trusted proxy address to ${{{CIDR_VAR}}} (or ${{{VAR}}}/32), not 0.0.0.0/0 or ::/0"
    );
    if official && m.permissions.contains(&Permission::NetworkEgress) {
        report.problems.push(format!(
            "{advice}. If an egress plugin can't do that, use OIDC or {{secret:…}} instead (ADR-0023)"
        ));
    } else if m.lan.is_some() {
        report.warnings.push(format!(
            "{advice}. Otherwise the LAN port of the web service stays closed by default; users have to turn on the lan_webui switch"
        ));
    } else {
        report.warnings.push(format!(
            "{advice}. If that's impossible, note it for the index review (ADR-0023)"
        ));
    }
}

/// 裸写会被 YAML 1.1 的工具（docker compose 等）当成布尔的环境变量值。
const BARE_BOOLEANS: [&str; 4] = ["on", "off", "yes", "no"];

/// 两处看原文才查得出来的清单问题：会被解析成布尔的环境变量值，和指不到服务的 `depends_on`。
fn check_compose_hygiene(package: &PluginPackage, report: &mut Report) {
    for (service, key, value) in bare_boolean_env(&package.source) {
        report.warnings.push(format!(
            "services.{service}.environment.{key} is a bare `{value}`: YAML 1.1 tools read it as a boolean and hand the container \"true\"/\"false\"; quote it when the app expects `{value}`"
        ));
    }
    let services: BTreeSet<&str> = package.service_names().collect();
    for (service, dependency) in missing_dependencies(&package.source) {
        if !services.contains(dependency.as_str()) {
            report.warnings.push(format!(
                "services.{service}.depends_on points at `{dependency}`, which isn't a service in this compose file: the install waits for a container that never exists"
            ));
        }
    }
}

/// 文本里 `services.<服务>.environment` 的每一项值恰好是 `on`、`off`、`yes`、`no`（没加引号）的地方：
/// （服务，键，值）。解析出来的值分不出原来写的是 `on` 还是 `true`，只能看原文。只看映射写法：列表写法
/// （`- KEY=值`）整条是字符串，不会被当成布尔。
fn bare_boolean_env(source: &str) -> Vec<(String, String, String)> {
    let mut found = Vec::new();
    let mut service = String::new();
    let mut in_environment = false;
    let mut item_indent = 0;
    for raw in source.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        // 服务名：`services:` 下的一级键。
        if indent == 2 && trimmed.ends_with(':') {
            service = trimmed
                .trim_end_matches(':')
                .trim_matches(['\'', '"'])
                .to_owned();
            in_environment = false;
            continue;
        }
        if service.is_empty() {
            continue;
        }
        if indent <= 4 {
            in_environment = trimmed.starts_with("environment:");
            item_indent = 0;
            continue;
        }
        if !in_environment {
            continue;
        }
        if item_indent == 0 {
            item_indent = indent;
        }
        if indent != item_indent {
            continue;
        }
        if trimmed.starts_with("- ") {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_owned(), value.trim());
        let value = value.split(" #").next().unwrap_or(value).trim();
        if BARE_BOOLEANS.contains(&value) {
            found.push((service.clone(), key, value.to_owned()));
        }
    }
    found
}

/// `depends_on` 指向的服务名（列表和长语法两种写法）。
fn missing_dependencies(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut service = String::new();
    let mut in_dependencies = false;
    let mut item_indent = 0;
    for raw in source.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent == 2 && trimmed.ends_with(':') {
            service = trimmed
                .trim_end_matches(':')
                .trim_matches(['\'', '"'])
                .to_owned();
            in_dependencies = false;
            continue;
        }
        if service.is_empty() {
            continue;
        }
        if indent <= 4 {
            in_dependencies = trimmed.starts_with("depends_on:");
            item_indent = 0;
            // 流式列表 `depends_on: [db, cache]` 写在一行里。
            if let Some((_, rest)) = trimmed.split_once('[')
                && let Some((names, _)) = rest.split_once(']')
                && in_dependencies
            {
                for name in names.split(',') {
                    let name = name.trim().trim_matches(['\'', '"']);
                    if !name.is_empty() {
                        found.push((service.clone(), name.to_owned()));
                    }
                }
            }
            continue;
        }
        if !in_dependencies {
            continue;
        }
        if item_indent == 0 {
            item_indent = indent;
        }
        if indent != item_indent {
            continue;
        }
        // 列表写法是名字全文；长语法 `名字: {condition: …}` 只取冒号前的名字。
        let name = trimmed
            .strip_prefix("- ")
            .map_or_else(
                || trimmed.split_once(':').map_or(trimmed, |(name, _)| name),
                |name| name,
            )
            .trim_matches(['\'', '"'])
            .to_owned();
        if !name.is_empty() {
            found.push((service.clone(), name));
        }
    }
    found
}

fn has_healthcheck(body: &ContainerCreateBody) -> bool {
    let test = body.healthcheck.as_ref().and_then(|h| h.test.as_ref());
    test.and_then(|t| t.first())
        .is_some_and(|first| first != "NONE")
}

async fn check_icon(
    package: &PluginPackage,
    official: bool,
    images: &ImageWorker,
    report: &mut Report,
) {
    let Some(raw) = &package.manifest.icon else {
        if official {
            report
                .problems
                .push("the official index requires an icon".to_owned());
        } else {
            report
                .warnings
                .push("no icon: the store and the home screen show a placeholder".to_owned());
        }
        return;
    };
    // 解析插件包时已经核对过路径、格式和文件。
    let path = package_files::normalize(raw).unwrap_or_else(|_| raw.clone());
    let (Some(format), Some(data)) = (IconFormat::from_path(&path), package.files().get(&path))
    else {
        report.problems.push(format!("icon {path} can't be read"));
        return;
    };
    match images.icon(format, data).await {
        Ok(icon) if icon.transparent_corner => report.warnings.push(format!(
            "icon {path} has transparent corners: icons must be full-bleed; the app rounds the corners"
        )),
        Ok(_) => {}
        Err(e) => report.problems.push(format!("icon {path}: {e}")),
    }
}

async fn check_tab_icons(package: &PluginPackage, images: &ImageWorker, report: &mut Report) {
    for (index, tab) in package
        .manifest
        .tabs
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        for (field, raw) in tab.icon_refs(index) {
            let path = package_files::normalize(raw).unwrap_or_else(|_| raw.to_owned());
            let (Some(format), Some(data)) =
                (IconFormat::from_path(&path), package.files().get(&path))
            else {
                report
                    .problems
                    .push(format!("{field} {path} can't be read"));
                continue;
            };
            if let Err(e) = images.tab_icon(format, data).await {
                report.problems.push(format!("{field} {path}: {e}"));
            }
        }
    }
}

fn check_tab_title_lengths(package: &PluginPackage, report: &mut Report) {
    for (index, tab) in package
        .manifest
        .tabs
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        for (locale, max) in [("zh", 4), ("en", 8)] {
            if tab
                .title
                .get(locale)
                .is_some_and(|title| title.chars().count() > max)
            {
                report.warnings.push(format!(
                    "tabs[{index}].title.{locale} exceeds the suggested length"
                ));
            }
        }
    }
}

/// 截图不算插件包的一部分，从插件包目录里读。
async fn check_screenshots(
    dir: BorrowedFd<'_>,
    manifest: &Manifest,
    images: &ImageWorker,
    report: &mut Report,
) {
    let mut landscape = BTreeSet::new();
    for raw in manifest.screenshots.iter().flatten() {
        let path = package_files::normalize(raw).unwrap_or_else(|_| raw.clone());
        let shot = match package_files::read_at(dir, &path) {
            Ok(data) => images.screenshot(&data).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match shot {
            Ok(shot) => {
                landscape.insert(shot.width >= shot.height);
                if shot.width.max(shot.height) < SCREENSHOT_EDGE {
                    report.warnings.push(format!(
                        "screenshot {path} is {}×{}; the long edge should be at least {SCREENSHOT_EDGE}",
                        shot.width, shot.height
                    ));
                }
            }
            Err(e) => report.problems.push(format!("screenshot {path}: {e}")),
        }
    }
    if landscape.len() > 1 {
        report
            .warnings
            .push("landscape and portrait screenshots are mixed".to_owned());
    }
}
