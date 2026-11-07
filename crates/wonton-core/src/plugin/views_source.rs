//! 安装卡片里和来源有关的提示。

use wonton_protocol::CardLine;

use super::github_source::Trust;
use super::languages::Languages;
use super::package::PluginPackage;
use super::render::Deployment;
use super::views::line;
use crate::l10n;

/// 来源相关的内容：审核状态、镜像来自哪些仓库、没按摘要固定的镜像。
pub(super) fn source_notices(
    package: &PluginPackage,
    plan: &Deployment,
    langs: &Languages,
) -> Vec<CardLine> {
    let unpinned: Vec<String> = plan
        .containers
        .iter()
        .filter(|c| !c.image.contains("@sha256:"))
        .map(|c| {
            l10n::text(
                langs,
                "install-image",
                &[
                    ("service", c.service.as_str().into()),
                    ("image", c.image.as_str().into()),
                ],
            )
        })
        .collect();
    let mut notices = Vec::new();
    let reviewed = package
        .origin
        .as_ref()
        .is_some_and(|o| o.trust == Trust::Official);
    if !reviewed {
        notices.push(CardLine {
            text: unreviewed_notice(package, langs),
            emphasis: true,
        });
        let images = image_repositories(plan.containers.iter().map(|c| c.image.as_str()));
        notices.push(line(l10n::text(
            langs,
            "install-images-from",
            &[("images", l10n::list(langs, &images).into())],
        )));
    }
    if !unpinned.is_empty() {
        notices.push(line(l10n::text(
            langs,
            "install-images-unpinned",
            &[("images", l10n::list(langs, &unpinned).into())],
        )));
    }
    notices
}

/// 未审核的插件的提示：写明谁提供。上传的插件只有清单里作者自己写的名字，核实不了。
fn unreviewed_notice(package: &PluginPackage, langs: &Languages) -> String {
    match &package.origin {
        Some(origin) => {
            let provider = origin.provider();
            match origin.via_index() {
                Some(host) => l10n::text(
                    langs,
                    "install-unreviewed-provider-via",
                    &[("provider", provider.into()), ("host", host.into())],
                ),
                None => l10n::text(
                    langs,
                    "install-unreviewed-provider",
                    &[("provider", provider.into())],
                ),
            }
        }
        None => match &package.manifest.author {
            Some(author) => l10n::text(
                langs,
                "install-unreviewed-upload-author",
                &[("author", author.name.as_str().into())],
            ),
            None => l10n::text(langs, "install-unreviewed-upload", &[]),
        },
    }
}

/// 镜像来自哪些仓库，去掉标签和摘要，去重，保持先后。没写主机的按 Docker 的规则算 `docker.io`。
fn image_repositories<'a>(images: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut repositories = Vec::new();
    for image in images {
        let repository = image_repository(image);
        if !repositories.contains(&repository) {
            repositories.push(repository);
        }
    }
    repositories
}

fn image_repository(image: &str) -> String {
    let name = image.split('@').next().unwrap_or(image);
    // 标签的冒号在最后一个 `/` 之后；主机的端口号在它之前。
    let last_part = name.rfind('/').map_or(0, |slash| slash + 1);
    let name = match name[last_part..].find(':') {
        Some(colon) => &name[..last_part + colon],
        None => name,
    };
    let first = name.split('/').next().unwrap_or(name);
    let has_host =
        name.contains('/') && (first.contains('.') || first.contains(':') || first == "localhost");
    if has_host {
        name.to_owned()
    } else if name.contains('/') {
        format!("docker.io/{name}")
    } else {
        format!("docker.io/library/{name}")
    }
}
