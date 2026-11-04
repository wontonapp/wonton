//! `wonton-core plugin check <目录> [--official]`（插件规范的“开发与校验”）：和安装时同一套解析和校验，再检查图标和
//! 截图（和核心一样在子进程里处理）；`--official` 再按官方索引的收录要求检查。最后给出插件包摘要。不连 Docker，也不联网。

use std::collections::BTreeSet;
use std::path::Path;

use bollard::models::ContainerCreateBody;

use super::core_address::VAR;
use super::egress::hardcoded_proxy;
use super::image_worker::ImageWorker;
use super::images::IconFormat;
use super::lan::webui::impersonable;
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
    let package = PluginPackage::load_dir(dir).map_err(|e| e.to_string())?;
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
            .push("kind: system 的插件只能随核心预装，主人装不了".to_owned());
    }
    for permission in m.permissions.iter().filter(|p| !supported(p)) {
        report
            .problems
            .push(format!("核心还不支持权限 {permission}，装不上"));
    }
    match dry_run(&package) {
        Ok(plan) => check_services(&plan, official, &mut report),
        Err(e) => report.problems.push(e.to_string()),
    }
    check_trusts_core(&package, official, &mut report);
    check_icon(&package, official, images, &mut report).await;
    check_screenshots(dir, m, images, &mut report).await;
    if official {
        let required = [
            ("category", m.category.is_some()),
            ("license", m.license.is_some()),
            ("author", m.author.is_some()),
            ("notices", !m.notices.is_empty()),
        ];
        for (field, _) in required.iter().filter(|(_, present)| !present) {
            report.problems.push(format!("官方索引要求写 {field}"));
        }
    }
    Ok(report)
}

/// 镜像要按摘要固定，长期运行的服务要有健康检查：官方索引要求，其他插件只提醒。只看 compose 里写的健康检查，
/// 镜像自带的看不到（检查不连 Docker）。
fn check_services(plan: &Deployment, official: bool, report: &mut Report) {
    let list = if official {
        &mut report.problems
    } else {
        &mut report.warnings
    };
    for container in &plan.containers {
        if !container.image.contains("@sha256:") {
            list.push(format!(
                "服务 {} 的镜像 {} 没有按摘要固定（写成 名字:标签@sha256:…）",
                container.service, container.image
            ));
        }
        if !container.one_shot && !has_healthcheck(&container.body) {
            list.push(format!(
                "服务 {} 没有健康检查，装上后状态显示“健康未知”",
                container.service
            ));
        }
        if official {
            let env = container.body.env.iter().flatten();
            for line in env.filter(|line| hardcoded_proxy(line)) {
                let name = line.split('=').next().unwrap_or_default();
                list.push(format!(
                    "服务 {} 的 {name} 写了具体的代理地址：核心会给出网插件注入代理变量，换一台主机这个地址也用不了（ADR-0023）",
                    container.service
                ));
            }
        }
    }
}

/// 用请求头认用户的应用，信任的代理地址只写核心的地址（ADR-0023 的“只信任核心的地址”）。出网插件能从宿主连到它，所以官方
/// 索引不收；不出网的只有开了局域网端口才有人连得到，那些端口有开关。
fn check_trusts_core(package: &PluginPackage, official: bool, report: &mut Report) {
    if !impersonable(package) {
        return;
    }
    let m = &package.manifest;
    let advice = format!(
        "网页用请求头认用户，但不是只信任核心：把信任的代理地址写成 ${{{VAR}}}，不写 0.0.0.0/0 和 ::/0"
    );
    if official && m.permissions.contains(&Permission::NetworkEgress) {
        report.problems.push(format!(
            "{advice}。出网插件做不到时改用 OIDC 或 {{secret:…}}（ADR-0023）"
        ));
    } else if m.lan.is_some() {
        report.warnings.push(format!(
            "{advice}。否则网页所在服务的局域网端口默认不开，要主人打开开关 lan_webui"
        ));
    } else {
        report.warnings.push(format!(
            "{advice}。做不到时要在索引的审核里登记（ADR-0023）"
        ));
    }
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
                .push("官方索引要求有图标（icon）".to_owned());
        } else {
            report
                .warnings
                .push("没有图标：商店和主屏上显示占位图标".to_owned());
        }
        return;
    };
    // 解析插件包时已经核对过路径、格式和文件。
    let path = package_files::normalize(raw).unwrap_or_else(|_| raw.clone());
    let (Some(format), Some(data)) = (IconFormat::from_path(&path), package.files().get(&path))
    else {
        report.problems.push(format!("图标 {path} 读不到"));
        return;
    };
    match images.icon(format, data).await {
        Ok(icon) if icon.transparent_corner => report.warnings.push(format!(
            "图标 {path} 的角是透明的：图标要满幅，圆角由 App 裁"
        )),
        Ok(_) => {}
        Err(e) => report.problems.push(format!("图标 {path}：{e}")),
    }
}

/// 截图不算插件包的一部分，从仓库目录里读。
async fn check_screenshots(
    dir: &Path,
    manifest: &Manifest,
    images: &ImageWorker,
    report: &mut Report,
) {
    let mut landscape = BTreeSet::new();
    for raw in manifest.screenshots.iter().flatten() {
        let path = package_files::normalize(raw).unwrap_or_else(|_| raw.clone());
        let shot = match package_files::read(dir, &path) {
            Ok(data) => images.screenshot(&data).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match shot {
            Ok(shot) => {
                landscape.insert(shot.width >= shot.height);
                if shot.width.max(shot.height) < SCREENSHOT_EDGE {
                    report.warnings.push(format!(
                        "截图 {path} 是 {}×{}，建议长边至少 {SCREENSHOT_EDGE}",
                        shot.width, shot.height
                    ));
                }
            }
            Err(e) => report.problems.push(format!("截图 {path}：{e}")),
        }
    }
    if landscape.len() > 1 {
        report
            .warnings
            .push("横屏和竖屏的截图混在了一起".to_owned());
    }
}
