//! 选哪家厂商的显卡（ADR-0099）：按 `requirements.gpu.vendors` 的书写顺序，取本机第一家能用的。
//!
//! 能用有三条：这家有可用的驱动（AMD、Intel 的 DRM 渲染节点，NVIDIA 的驱动加 Docker 的 `nvidia` 运行时）、
//! `gpu.images` 里这家的镜像支持本机架构、这家确实在插件支持的列表里。镜像的平台列表经 Docker 读，不拉镜像；
//! 读不到时算能用，预览里如实写查不出来。
//!
//! 选中之后：只给这家设备的，照 `gpu.images` 换上这家的镜像；名单里别家照旧用通用镜像。一家都不行时，
//! `required` 装不上，`optional` 用通用镜像、不给设备。装过以后照安装记录里那家来，不重新选（见 [`GpuGrant`]）。

use super::gpu::{GpuGrant, GpuImage, GpuProblem, probe, requirement};
use super::host_check;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{GpuNeed, Manifest, Permission};
use super::requirements::{GpuVendor, card_name};
use crate::l10n::{list, text};

impl PluginManager {
    /// 照清单的顺序选一家能用的厂商。都不行时给原因：这台主机的显卡不在支持的列表里，或者每家的镜像都有问题。
    async fn choose_gpu(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<Result<GpuGrant, GpuProblem>, InstallError> {
        let wanted: Vec<GpuVendor> = manifest
            .requirements
            .as_ref()
            .and_then(|requirements| requirements.gpu.as_ref())
            .map_or_else(Vec::new, |gpu| gpu.vendors.clone());
        let probed = match probe(&self.host, langs) {
            Ok(probed) => probed,
            Err(problem) => return Ok(Err(problem)),
        };
        let host = card_name(&probed.vendors.iter().copied().collect::<Vec<_>>(), langs);
        // 不写 `vendors` 等于哪家都行，按探测到的顺序取。
        let candidates: Vec<GpuVendor> = if wanted.is_empty() {
            probed.vendors.iter().copied().collect()
        } else {
            wanted
                .iter()
                .copied()
                .filter(|vendor| probed.vendors.contains(vendor))
                .collect()
        };
        let nvidia_runtime = || async {
            Ok::<_, InstallError>(
                self.engine
                    .runtimes()
                    .await?
                    .iter()
                    .any(|runtime| runtime == "nvidia"),
            )
        };
        let mut reasons = Vec::new();
        for vendor in candidates {
            if vendor == GpuVendor::Nvidia && !nvidia_runtime().await? {
                reasons.push(text(langs, "plugin-install-gpu-problem-no-runtime", &[]));
                continue;
            }
            let mut images = self.vendor_images(manifest, vendor);
            // 没有厂商版镜像的插件不读宿主的架构。
            let host_arch = if images.is_empty() {
                String::new()
            } else {
                self.engine.host_arch().await?
            };
            let mut arch_unknown = Vec::new();
            let mut supported = true;
            for image in &mut images {
                match self.engine.image_platforms(&image.image).await {
                    Ok(platforms) => {
                        if !platforms.supports(&host_arch) {
                            reasons.push(text(
                                langs,
                                "plugin-install-gpu-problem-image-arch",
                                &[
                                    ("service", image.service.as_str().into()),
                                    ("image", image.image.as_str().into()),
                                    ("arch", host_arch.as_str().into()),
                                ],
                            ));
                            supported = false;
                            break;
                        }
                        image.size = platforms.compressed_size();
                    }
                    // 读不到（离线、仓库拒绝）不拦安装：拉不下来时安装自己会报 image_pull_failed。
                    Err(error) => arch_unknown.push(text(
                        langs,
                        "plugin-install-gpu-problem-image-unknown",
                        &[
                            ("image", image.image.as_str().into()),
                            ("error", error.to_string().into()),
                        ],
                    )),
                }
            }
            if !supported {
                continue;
            }
            let mut grant = probed.clone().keep(vendor);
            grant.images = images;
            grant.arch_unknown = arch_unknown;
            return Ok(Ok(grant));
        }
        let wanted = card_name(&wanted, langs);
        Ok(Err(GpuProblem::NoWantedVendor {
            host,
            wanted,
            reasons,
        }))
    }

    /// 这次安装照清单选一家厂商（见 [`Self::choose_gpu`]）。
    async fn probe_gpu(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<Result<GpuGrant, GpuProblem>, InstallError> {
        self.choose_gpu(manifest, langs).await
    }

    /// 给插件的显卡，或者给不了的原因：先按它的要求判断（厂商、显存），再看设备实际能不能交出去。
    async fn gpu_fit(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<Result<GpuGrant, String>, InstallError> {
        if let Some(problem) =
            host_check::gpu_problem(manifest, &self.host_facts(langs).await, langs)
        {
            return Ok(Err(problem));
        }
        Ok(self
            .probe_gpu(manifest, langs)
            .await?
            .map_err(|p| p.message(langs)))
    }

    /// 申请了 `devices.gpu` 的插件在这台主机上拿到什么：没有符合的显卡时，`required` 的装不了（原因写清），
    /// `optional` 的照常装、不给设备。没申请的返回 `None`，不碰 Docker。
    pub(super) async fn gpu_grant(
        &self,
        manifest: &Manifest,
        granted: &[Permission],
        langs: &Languages,
    ) -> Result<Option<GpuGrant>, InstallError> {
        let Some(requirement) = requirement(manifest) else {
            return Ok(None);
        };
        if !granted.contains(&Permission::DevicesGpu) {
            return Ok(None);
        }
        match self.gpu_fit(manifest, langs).await? {
            Ok(grant) => Ok(Some(grant)),
            Err(problem) if requirement == GpuNeed::Optional => {
                tracing::info!(plugin = %manifest.id, %problem, "本机没有符合的显卡，插件不带显卡安装");
                Ok(None)
            }
            Err(problem) => Err(InstallError::GpuUnavailable(problem)),
        }
    }

    /// 重建、重启、核心升级用安装记录里选中的那家，不重新选（ADR-0099）。本机现在没有这家时镜像照换、设备一个
    /// 不给，由容器自己报错。
    pub(super) async fn gpu_grant_recorded(
        &self,
        manifest: &Manifest,
        granted: &[Permission],
        vendor: Option<GpuVendor>,
        langs: &Languages,
    ) -> Result<Option<GpuGrant>, InstallError> {
        let Some(vendor) = vendor else {
            return Ok(None);
        };
        if requirement(manifest).is_none() || !granted.contains(&Permission::DevicesGpu) {
            return Ok(None);
        }
        let images = self.vendor_images(manifest, vendor);
        let probed = match probe(&self.host, langs) {
            Ok(probed) => probed,
            Err(_) => return Ok(Some(GpuGrant::images_only(vendor, images))),
        };
        if !probed.vendors.contains(&vendor) {
            return Ok(Some(GpuGrant::images_only(vendor, images)));
        }
        let mut grant = probed.keep(vendor);
        grant.images = images;
        Ok(Some(grant))
    }

    /// 清单里这家厂商换上的镜像，按服务名排序。没写 `gpu.images` 时为空（全用通用镜像）。
    fn vendor_images(&self, manifest: &Manifest, vendor: GpuVendor) -> Vec<GpuImage> {
        manifest
            .gpu
            .as_ref()
            .into_iter()
            .flat_map(|gpu| &gpu.images)
            .filter_map(|(service, by_vendor)| {
                by_vendor.get(&vendor).map(|image| GpuImage {
                    service: service.clone(),
                    image: image.clone(),
                    size: None,
                })
            })
            .collect()
    }

    /// 清单要显卡时，预览里关于显卡的那一行：选中的厂商、设备、镜像，或者为什么给不了。`required` 而给不了时
    /// 同时是装不了的原因。
    pub(super) async fn gpu_preview(
        &self,
        manifest: &Manifest,
        enabled: bool,
        langs: &Languages,
    ) -> Result<super::gpu::GpuPreview, InstallError> {
        let Some(requirement) = requirement(manifest) else {
            return Ok(super::gpu::GpuPreview::none());
        };
        if !enabled {
            return Ok(super::gpu::GpuPreview::none());
        }
        let services = manifest
            .gpu
            .as_ref()
            .map(|g| list(langs, &g.services))
            .unwrap_or_default();
        let (line, blocked) = match self.gpu_fit(manifest, langs).await? {
            Ok(grant) => (
                text(
                    langs,
                    "plugin-install-gpu-preview-granted",
                    &[
                        ("services", services.as_str().into()),
                        ("describe", grant.describe(langs).into()),
                    ],
                ),
                None,
            ),
            Err(problem) if requirement == GpuNeed::Optional => (
                text(
                    langs,
                    "plugin-install-gpu-preview-missing",
                    &[
                        ("problem", problem.as_str().into()),
                        ("services", services.as_str().into()),
                    ],
                ),
                None,
            ),
            Err(problem) => {
                let reason = InstallError::GpuUnavailable(problem).to_string();
                (
                    text(
                        langs,
                        "plugin-install-gpu-preview-required",
                        &[
                            ("services", services.as_str().into()),
                            ("reason", reason.as_str().into()),
                        ],
                    ),
                    Some(reason),
                )
            }
        };
        Ok(super::gpu::GpuPreview::line(line, blocked))
    }
}
