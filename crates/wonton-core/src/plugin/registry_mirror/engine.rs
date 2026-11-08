//! 镜像由 Docker 按摘要核对内容；拉完还要核对它登记的 RepoDigests。

use crate::l10n;

use bollard::errors::Error as BollardError;
use bollard::query_parameters::CreateImageOptionsBuilder;
use futures_util::StreamExt;

use super::reference::{digest, mirrored, pull_reference};
use crate::plugin::compose::PullPolicy;
use crate::plugin::docker::{Engine, EngineError};

type Result<T, E = EngineError> = std::result::Result<T, E>;

/// 拉镜像时一层报到的情况。Docker 的 `progressDetail` 是按层的，百分比要按层的字节数加起来算。
pub(in crate::plugin) struct LayerProgress<'a> {
    /// 层的 ID。
    pub id: &'a str,
    /// 这一层已经下完，或者本来就在本地（`Already exists`、`Pull complete`）。
    pub done: bool,
    /// 这一层下到的字节数。
    pub current: u64,
    pub total: u64,
}

impl Engine {
    pub async fn ensure_image(
        &self,
        image: &str,
        policy: PullPolicy,
        mirror: Option<&str>,
    ) -> Result<String> {
        self.ensure_image_with_progress(image, policy, mirror, &mut |_layer: LayerProgress<'_>| {})
            .await
    }

    /// 同 [`Self::ensure_image`]。拉的时候每层的字节数交给 `progress`；本机已经有了、不拉时不报。
    pub(in crate::plugin) async fn ensure_image_with_progress(
        &self,
        image: &str,
        policy: PullPolicy,
        mirror: Option<&str>,
        progress: &mut (dyn FnMut(LayerProgress<'_>) + Send),
    ) -> Result<String> {
        let accelerated = mirror.and_then(|mirror| mirrored(image, mirror));
        if policy != PullPolicy::Always {
            if self.image_present(image).await? {
                return Ok(image.to_owned());
            }
            if let Some(reference) = &accelerated
                && self.image_present(reference).await?
            {
                self.check_digest(reference).await?;
                return Ok(reference.clone());
            }
            if policy == PullPolicy::Never {
                let langs = self.owner_languages();
                return Err(EngineError::NotFound(l10n::text(
                    &langs,
                    "plugin-settings-mirror-policy-never",
                    &[("image", image.into())],
                )));
            }
        }
        let reference = accelerated.as_deref().unwrap_or(image);
        let pulled = async {
            self.pull(reference, progress).await?;
            if digest(reference).is_some() {
                self.check_digest(reference).await?;
            }
            Ok::<_, EngineError>(())
        }
        .await;
        if let Err(error) = pulled {
            return Err(if accelerated.is_some() {
                let langs = self.owner_languages();
                EngineError::Pull {
                    image: reference.to_owned(),
                    message: l10n::text(
                        &langs,
                        "plugin-settings-mirror-pull-failed",
                        &[
                            ("mirror", mirror.expect("改写引用有加速地址").into()),
                            ("error", error.to_string().into()),
                        ],
                    ),
                }
            } else {
                error
            });
        }
        if accelerated.is_some() && self.image_present(image).await? {
            Ok(image.to_owned())
        } else {
            Ok(reference.to_owned())
        }
    }

    async fn image_present(&self, image: &str) -> Result<bool> {
        match self.docker.inspect_image(image).await {
            Ok(_) => Ok(true),
            // containerd 只有别的架构变体时 Config/RootFS 为空；补拉本机变体后才能读取健康检查。
            Err(BollardError::JsonDataError { .. }) => Ok(false),
            Err(error) => {
                let langs = self.owner_languages();
                let error = crate::plugin::docker::from_bollard(
                    &langs,
                    &crate::l10n::text(&langs, "plugin-settings-mirror-inspect", &[]),
                    error,
                );
                match error {
                    EngineError::NotFound(_) => Ok(false),
                    other => Err(other),
                }
            }
        }
    }

    async fn check_digest(&self, image: &str) -> Result<()> {
        let expected = digest(image).expect("调用方已检查摘要");
        let langs = self.owner_languages();
        let inspect = self.docker.inspect_image(image).await.map_err(|error| {
            crate::plugin::docker::from_bollard(
                &langs,
                &crate::l10n::text(
                    &langs,
                    "plugin-settings-mirror-inspect-image",
                    &[("image", image.into())],
                ),
                error,
            )
        })?;
        let found = inspect
            .repo_digests
            .unwrap_or_default()
            .iter()
            .any(|reference| {
                reference
                    .rsplit_once('@')
                    .is_some_and(|(_, digest)| digest == expected)
            });
        if !found {
            let langs = self.owner_languages();
            return Err(EngineError::Pull {
                image: image.to_owned(),
                message: l10n::text(
                    &langs,
                    "plugin-settings-mirror-digest-mismatch",
                    &[("digest", expected.into())],
                ),
            });
        }
        Ok(())
    }

    async fn pull(
        &self,
        image: &str,
        progress: &mut (dyn FnMut(LayerProgress<'_>) + Send),
    ) -> Result<()> {
        let reference = pull_reference(image);
        let options = CreateImageOptionsBuilder::default()
            .from_image(&reference)
            .build();
        let mut stream = self.docker.create_image(Some(options), None, None);
        while let Some(item) = stream.next().await {
            let item = item.map_err(|error| match error {
                BollardError::DockerResponseServerError { message, .. }
                | BollardError::DockerStreamError { error: message } => EngineError::Pull {
                    image: image.to_owned(),
                    message: crate::l10n::text(
                        &self.owner_languages(),
                        "plugin-install-docker-pull-failed",
                        &[("image", image.into()), ("error", message.as_str().into())],
                    ),
                },
                other => {
                    let langs = self.owner_languages();
                    crate::plugin::docker::from_bollard(
                        &langs,
                        &crate::l10n::text(
                            &langs,
                            "plugin-settings-mirror-pull-image",
                            &[("image", image.into())],
                        ),
                        other,
                    )
                }
            })?;
            let Some(id) = item.id else { continue };
            let done = matches!(
                item.status.as_deref(),
                Some("Already exists" | "Pull complete" | "Download complete")
            );
            let (current, total) = item.progress_detail.map_or((0, 0), |detail| {
                (
                    detail.current.unwrap_or_default().max(0) as u64,
                    detail.total.unwrap_or_default().max(0) as u64,
                )
            });
            if done || total > 0 {
                progress(LayerProgress {
                    id: &id,
                    done,
                    current,
                    total,
                });
            }
        }
        Ok(())
    }

    pub(crate) async fn registry_proxy_configured(&self) -> Result<bool> {
        let info = self.docker.info().await.map_err(|error| {
            let langs = self.owner_languages();
            crate::plugin::docker::from_bollard(
                &langs,
                &crate::l10n::text(&langs, "plugin-settings-mirror-docker-proxy", &[]),
                error,
            )
        })?;
        Ok(info
            .https_proxy
            .is_some_and(|proxy| !proxy.trim().is_empty()))
    }
}
