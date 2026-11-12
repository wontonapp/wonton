//! 查新版本（ADR-0025）：经 Docker 的分发接口取渠道 tag 指向的 digest，发现新的就下载核心镜像，从镜像读版本号和不可逆的
//! 迁移。下载好才能点升级：确认页显示的因此总是真的。

use semver::Version;
use wonton_protocol::{
    CoreUpgradeCheck, CoreUpgradeFailure as Failure, CoreUpgradeOffer, CoreUpgradeProgress,
};

use super::host::{HostError, ImageInfo, Progress};
use super::service::{Running, Upgrades};
use super::state::problem;
use super::{LABEL_MIGRATIONS, LABEL_VERSION, state};

/// 后台自动下载至少要留这么多磁盘（已定 1）。
const AUTO_DOWNLOAD_MIN_FREE: u64 = 5_000_000_000;
/// 手动下载时镜像大小的余量（ADR-0020 同样的 5%）。
const SIZE_MARGIN_PERCENT: u64 = 5;

/// `WONTON_CORE_SOURCE` 是 `仓库:tag` 时拆开。digest 写死的、没写 tag 的不跟渠道。
pub fn channel(source: &str) -> Option<(&str, &str)> {
    if source.contains('@') {
        return None;
    }
    let (repository, tag) = source.rsplit_once(':')?;
    // `localhost:5000/core` 里的冒号是端口。
    (!tag.contains('/')).then_some((repository, tag))
}

pub fn version_of(info: &ImageInfo) -> Option<String> {
    info.labels.get(LABEL_VERSION).cloned()
}

/// 严格比当前旧才算旧：说不清时（没有版本号）不算。只升不降。
fn is_older(candidate: Option<&str>, current: Option<&str>) -> bool {
    let parse = |v: Option<&str>| v.and_then(|v| Version::parse(v.trim_start_matches('v')).ok());
    matches!((parse(candidate), parse(current)), (Some(c), Some(r)) if c < r)
}

/// 从 `current` 升到这个镜像会迁移、退不回去的卷。`current` 比声明的版本旧才算；当前版本没有版本号（开发版）时，声明的
/// 都算。标签读不懂的那一项也算：宁可多一条提示。
pub fn migrated_volumes(
    labels: &std::collections::HashMap<String, String>,
    current: Option<&str>,
) -> Vec<String> {
    let Some(declared) = labels.get(LABEL_MIGRATIONS) else {
        return Vec::new();
    };
    let current = current.and_then(|v| Version::parse(v.trim_start_matches('v')).ok());
    let mut volumes: Vec<String> = declared
        .split(',')
        .filter_map(|item| {
            let (volume, since) = item.trim().split_once('@').unwrap_or((item.trim(), ""));
            if volume.is_empty() {
                return None;
            }
            let crossed = match (&current, Version::parse(since)) {
                (Some(current), Ok(since)) => *current < since,
                _ => true,
            };
            crossed.then(|| volume.to_owned())
        })
        .collect();
    volumes.sort();
    volumes.dedup();
    volumes
}

impl Upgrades {
    /// 查一次，结果记下来。升级中不查（ADR-0025）。
    pub async fn check_now(&self, manual: bool) {
        let result = self.check_once(manual).await;
        self.set_check(result);
    }

    async fn check_once(&self, manual: bool) -> CoreUpgradeCheck {
        let Some(source) = self.config.source.as_deref() else {
            return CoreUpgradeCheck::NotInstalled;
        };
        let Some((repository, _)) = channel(source) else {
            return CoreUpgradeCheck::Pinned;
        };
        let running = match self.running().await {
            Ok(running) => running,
            Err(e) => {
                return CoreUpgradeCheck::Failed {
                    problem: problem(Failure::Upstream, format!("查核心自己的容器失败：{e}")),
                };
            }
        };
        let digest = match self.host.registry_digest(source).await {
            Ok(digest) => digest,
            Err(e) => {
                let message = format!("查不到 {source} 的最新版本：{e}");
                return CoreUpgradeCheck::Failed {
                    problem: problem(Failure::RegistryUnavailable, message),
                };
            }
        };
        if running.digest.as_deref() == Some(digest.as_str()) {
            self.forget_predownload(&running, None).await;
            return CoreUpgradeCheck::UpToDate;
        }
        let reference = format!("{repository}@{digest}");
        let info = match self.host.image(&reference).await {
            Ok(Some(info)) => info,
            Ok(None) => match self.download(&reference, &digest, &running, manual).await {
                Ok(info) => info,
                Err(result) => return *result,
            },
            Err(e) => {
                return CoreUpgradeCheck::DownloadFailed {
                    digest,
                    problem: problem(Failure::Upstream, format!("查镜像失败：{e}")),
                };
            }
        };
        self.offer(reference, digest, &info, &running).await
    }

    async fn offer(
        &self,
        reference: String,
        digest: String,
        info: &ImageInfo,
        running: &Running,
    ) -> CoreUpgradeCheck {
        let version = version_of(info);
        if is_older(version.as_deref(), running.version.as_deref()) {
            tracing::info!(%reference, ?version, current = ?running.version, "渠道上的版本比正在运行的旧，不算新版本");
            if let Err(e) = self.host.remove_image(&reference).await {
                tracing::warn!(%reference, error = %e, "删除比当前旧的镜像失败");
            }
            return CoreUpgradeCheck::UpToDate;
        }
        self.forget_predownload(running, Some(&reference)).await;
        CoreUpgradeCheck::Ready {
            offer: CoreUpgradeOffer {
                digest,
                version,
                migrates: migrated_volumes(&info.labels, running.version.as_deref()),
            },
        }
    }

    /// 下载前先看磁盘。放不下、拉不到时如实说，打开升级页或者手动检查时再试。
    async fn download(
        &self,
        reference: &str,
        digest: &str,
        running: &Running,
        manual: bool,
    ) -> Result<ImageInfo, Box<CoreUpgradeCheck>> {
        let failed = |reason, message: String| {
            Box::new(CoreUpgradeCheck::DownloadFailed {
                digest: digest.to_owned(),
                problem: problem(reason, message),
            })
        };
        // 查不到剩余空间时不拦：Docker 的存储可能在别的分区，拉取报错时如实报告。
        if let Ok(free) = self.host.free_bytes(&self.layout().data_dir) {
            if !manual && free < AUTO_DOWNLOAD_MIN_FREE {
                return Err(failed(
                    Failure::NoSpace,
                    "磁盘剩不到 5 GB，没有自动下载".to_owned(),
                ));
            }
            // 分发接口不给镜像大小，按当前核心镜像的大小估算。
            let needed = running.size + running.size / 100 * SIZE_MARGIN_PERCENT;
            if manual && free < needed {
                let message = format!("磁盘不够：下载大约要 {needed} 字节，只有 {free} 字节可用");
                return Err(failed(Failure::NoSpace, message));
            }
        }
        self.set_check(CoreUpgradeCheck::Downloading {
            digest: digest.to_owned(),
            progress: None,
        });
        let report = |progress: Progress| {
            self.set_check(CoreUpgradeCheck::Downloading {
                digest: digest.to_owned(),
                progress: Some(CoreUpgradeProgress {
                    done_bytes: progress.done,
                    total_bytes: progress.total,
                }),
            });
        };
        self.host
            .pull(reference, &report)
            .await
            .map_err(|e| failed(Failure::RegistryUnavailable, format!("下载不了：{e}")))?;
        match self.host.image(reference).await {
            Ok(Some(info)) => Ok(info),
            Ok(None) => Err(failed(
                Failure::Upstream,
                format!("拉完本机还是没有 {reference}"),
            )),
            Err(e) => Err(failed(Failure::Upstream, format!("查镜像失败：{e}"))),
        }
    }

    /// 只留一个预下载的版本。不删正在运行的、回滚要用的和这次升级的目标。
    async fn forget_predownload(&self, running: &Running, keep: Option<&str>) {
        let path = self.layout().upgrade_dir().join("predownload");
        let previous = std::fs::read_to_string(&path).unwrap_or_default();
        let previous = previous.trim();
        if !previous.is_empty() && Some(previous) != keep && !self.image_in_use(previous, running) {
            match self.host.remove_image(previous).await {
                Ok(()) | Err(HostError::NotFound(_)) => {}
                Err(e) => tracing::warn!(image = %previous, error = %e, "删除之前预下载的镜像失败"),
            }
        }
        let written = match keep {
            Some(reference) => state::write_atomic(&path, reference.as_bytes(), 0o600),
            None => state::write_atomic(&path, b"", 0o600),
        };
        if let Err(e) = written {
            tracing::warn!(error = %format!("{e:#}"), "记下预下载的镜像失败");
        }
    }

    fn image_in_use(&self, image: &str, running: &Running) -> bool {
        if running.config_image == image {
            return true;
        }
        match state::load(&self.layout().state_path()) {
            Ok(Some(last)) => {
                last.from.image == image
                    || last.to.image == image
                    || last.rollback_images.iter().any(|i| i == image)
            }
            Ok(None) => false,
            // 读不懂就不知道回滚要不要它：不删。
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn only_a_tag_follows_a_channel() {
        assert_eq!(
            channel("ghcr.io/x/core:beta"),
            Some(("ghcr.io/x/core", "beta"))
        );
        assert_eq!(
            channel("localhost:5000/core:stable"),
            Some(("localhost:5000/core", "stable"))
        );
        assert_eq!(channel("localhost:5000/core"), None);
        assert_eq!(channel("ghcr.io/x/core@sha256:abc"), None);
    }

    #[test]
    fn a_channel_never_downgrades() {
        assert!(is_older(Some("0.2.0"), Some("0.3.0")));
        assert!(is_older(Some("v0.3.0-beta.1"), Some("0.3.0-beta.2")));
        assert!(!is_older(Some("0.3.0"), Some("0.3.0")));
        assert!(!is_older(Some("0.4.0"), Some("0.3.0")));
        assert!(!is_older(None, Some("0.3.0")));
        assert!(!is_older(Some("0.4.0"), None));
    }

    /// 标签只增不减：跳版本时不会漏。
    #[test]
    fn a_migration_applies_only_when_the_current_version_is_older_than_it() {
        let labels = HashMap::from([(
            LABEL_MIGRATIONS.to_owned(),
            "tuwunel@0.5.0,hydra@0.6.0".to_owned(),
        )]);
        assert_eq!(
            migrated_volumes(&labels, Some("0.4.9")),
            ["hydra", "tuwunel"]
        );
        assert_eq!(migrated_volumes(&labels, Some("0.5.0")), ["hydra"]);
        assert_eq!(
            migrated_volumes(&labels, Some("0.6.0")),
            Vec::<String>::new()
        );
        assert_eq!(migrated_volumes(&labels, None), ["hydra", "tuwunel"]);
        assert_eq!(
            migrated_volumes(&HashMap::new(), None),
            Vec::<String>::new()
        );
    }
}
