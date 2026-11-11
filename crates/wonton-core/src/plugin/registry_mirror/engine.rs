//! 镜像由 Docker 按摘要核对内容；拉完还要核对它登记的 RepoDigests。

use crate::l10n;

use bollard::errors::Error as BollardError;
use bollard::query_parameters::CreateImageOptionsBuilder;
use futures_util::StreamExt;

use super::reference::{digest, pull_reference, station};
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
        accelerated: &[String],
    ) -> Result<String> {
        self.ensure_image_with_progress(image, policy, accelerated, &mut |_layer: LayerProgress<
            '_,
        >| {})
            .await
    }

    /// 同 [`Self::ensure_image`]。拉的时候每层的字节数交给 `progress`；本机已经有了、不拉时不报。
    /// `accelerated` 是同一个引用按名单顺序改写出的加速引用，一个站失败就换下一个（ADR-0074）。
    pub(in crate::plugin) async fn ensure_image_with_progress(
        &self,
        image: &str,
        policy: PullPolicy,
        accelerated: &[String],
        progress: &mut (dyn FnMut(LayerProgress<'_>) + Send),
    ) -> Result<String> {
        if policy != PullPolicy::Always {
            if self.image_present(image).await? && self.digest_matches(image).await {
                return Ok(image.to_owned());
            }
            for reference in accelerated {
                if self.image_present(reference).await?
                    && self.check_digest(reference).await.is_ok()
                {
                    return Ok(reference.clone());
                }
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
        if accelerated.is_empty() {
            self.pull_and_check(image, progress).await?;
            return Ok(image.to_owned());
        }
        let mut failures = Vec::new();
        for reference in accelerated {
            match self.pull_and_check(reference, progress).await {
                Ok(()) => {
                    return Ok(if self.image_present(image).await? {
                        image.to_owned()
                    } else {
                        reference.clone()
                    });
                }
                Err(error) => failures.push((station(reference).to_owned(), error.to_string())),
            }
        }
        Err(self.all_stations_failed(image, &failures))
    }

    async fn pull_and_check(
        &self,
        reference: &str,
        progress: &mut (dyn FnMut(LayerProgress<'_>) + Send),
    ) -> Result<()> {
        self.pull(reference, progress).await?;
        if digest(reference).is_some() {
            self.check_digest(reference).await?;
        }
        Ok(())
    }

    fn all_stations_failed(&self, image: &str, failures: &[(String, String)]) -> EngineError {
        let langs = self.owner_languages();
        let tries = failures
            .iter()
            .map(|(mirror, error)| {
                l10n::text(
                    &langs,
                    "plugin-settings-mirror-try-failed",
                    &[
                        ("mirror", mirror.as_str().into()),
                        ("error", error.as_str().into()),
                    ],
                )
            })
            .collect::<Vec<_>>();
        EngineError::Pull {
            image: image.to_owned(),
            message: l10n::text(
                &langs,
                "plugin-settings-mirror-all-failed",
                &[
                    ("image", image.into()),
                    ("tries", l10n::list(&langs, &tries).into()),
                ],
            ),
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

    /// 引用带摘要时要求本机那份的内容也是这个摘要：只按标签拉到过、标签换了指向的镜像照样算没有。
    async fn digest_matches(&self, image: &str) -> bool {
        digest(image).is_none() || self.check_digest(image).await.is_ok()
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
