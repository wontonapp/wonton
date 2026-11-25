use std::collections::{BTreeMap, BTreeSet};

use wonton_protocol::{InstallRequest, PluginScope, SecretValues};

use super::{InstallError, Languages, PluginManager, generic, service};
use crate::l10n;
use crate::plugin::installation::InstallDecision;
use crate::plugin::manifest::ManifestExt;
use crate::plugin::package::PluginPackage;
use crate::plugin::secrets::Secrets;
use crate::plugin::store::OpenedBy;

pub(super) struct PreparedInstall {
    pub decision: InstallDecision,
    pub inputs: SecretValues,
    /// 这一层包设置经 schema 校验后的值，装和部署用同一份。
    pub settings: std::collections::BTreeMap<String, serde_json::Value>,
    pub secrets_before: Option<Secrets>,
    pub changed_before: BTreeMap<String, u64>,
}

impl PluginManager {
    pub(super) async fn prepare_install_batch(
        &self,
        package: PluginPackage,
        request: InstallRequest,
        retained: Option<&[PluginPackage]>,
        langs: &Languages,
    ) -> Result<Vec<PreparedInstall>, InstallError> {
        let preview = if retained.is_none() {
            Some(self.dependency_preview(&package.manifest, langs).await?)
        } else {
            None
        };
        // 计划绑定的是依赖图的连接集合：只有触及依赖图的请求（消费方声明了能力，或随批安装提供方）
        // 才要求原样带回计划；没有能力声明的旧调用方（如商店直装）只校验包摘要。
        let enforce_plan = super::declarations(&package.manifest).next().is_some();
        let mut consumer = self
            .prepare_install_decision(package, &request, enforce_plan, langs)
            .await?;
        let mut ids = BTreeSet::from([consumer.decision.package.manifest.id.clone()]);
        let candidates = if request.install_together.is_empty() || retained.is_some() {
            Vec::new()
        } else {
            self.dependency_candidates(langs).await?
        };
        if retained.is_some_and(|packages| packages.len() != request.install_together.len()) {
            return Err(selection_error(langs));
        }
        let mut providers = Vec::with_capacity(request.install_together.len());
        for (index, requested) in request.install_together.iter().enumerate() {
            let inner = &requested.request;
            if !inner.install_together.is_empty() || !inner.connect_plugins.is_empty() {
                return Err(selection_error(langs));
            }
            let package = self
                .resolve_provider_package(
                    requested,
                    retained.map(|packages| (packages, index)),
                    &candidates,
                    preview.as_ref(),
                    langs,
                )
                .await?;
            if package.digest() != inner.digest {
                return Err(InstallError::Changed(l10n::text(
                    langs,
                    "plugin-capability-plan-changed",
                    &[],
                )));
            }
            if package.manifest.id.as_str() != requested.id
                || !ids.insert(package.manifest.id.clone())
            {
                return Err(selection_error(langs));
            }
            if !package.manifest.requires.is_empty() {
                return Err(selection_error(langs));
            }
            providers.push(
                self.prepare_install_decision(package, inner, enforce_plan, langs)
                    .await?,
            );
        }
        for capability in consumer
            .decision
            .package
            .manifest
            .requires
            .iter()
            .filter(|capability| generic(capability))
        {
            let targets = consumer
                .decision
                .targets
                .as_mut()
                .expect("安装决定固定连接集合");
            if targets.contains_key(capability) {
                continue;
            }
            let provider = providers
                .iter()
                .find(|provider| service(&provider.decision.package.manifest, capability).is_some())
                .ok_or_else(|| selection_error(langs))?;
            targets.insert(
                capability.clone(),
                provider.decision.package.manifest.id.to_string(),
            );
        }
        self.check_batch_slugs(&providers, &consumer, langs)?;
        self.validate_connect_plugins(
            &consumer.decision.package.manifest,
            &request.connect_plugins,
            langs,
        )?;
        providers.push(consumer);
        Ok(providers)
    }

    /// 重试用核心保留的原包；首次按预览里缺的能力选候选，再取来源并核对摘要与出处一致。
    async fn resolve_provider_package(
        &self,
        requested: &wonton_protocol::InstallTogether,
        retained: Option<(&[PluginPackage], usize)>,
        candidates: &[super::preview::Candidate],
        preview: Option<&super::preview::DependencyPreview>,
        langs: &Languages,
    ) -> Result<PluginPackage, InstallError> {
        let Some((packages, index)) = retained else {
            let candidate = candidates
                .iter()
                .find(|candidate| {
                    candidate.listed.manifest.id.as_str() == requested.id
                        && preview
                            .as_ref()
                            .expect("首次安装有预览")
                            .missing_requires
                            .iter()
                            .any(|missing| {
                                missing.providers.iter().any(|provider| {
                                    provider.id == requested.id
                                        && provider.source == candidate.source()
                                })
                            })
                })
                .ok_or_else(|| selection_error(langs))?;
            let package = self.fetch_source(&requested.request.source).await?;
            let expected = candidate.files.package(&candidate.listed, langs).await?;
            if package.digest() != expected.digest()
                || package.origin.as_ref().is_none_or(|origin| {
                    origin.repo != candidate.origin.repo
                        || origin.commit != candidate.origin.commit
                        || origin.index != candidate.origin.index
                })
            {
                return Err(selection_error(langs));
            }
            return Ok(package);
        };
        Ok(packages[index].clone())
    }

    fn check_batch_slugs(
        &self,
        providers: &[PreparedInstall],
        consumer: &PreparedInstall,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut slugs = BTreeSet::new();
        for layer in providers.iter().chain(std::iter::once(consumer)) {
            let id = &layer.decision.package.manifest.id;
            let slug = id.slug();
            if let Some(other) = plugins.keys().find(|other| other.slug() == slug) {
                return Err(InstallError::Conflict(l10n::text(
                    langs,
                    "plugin-install-conflict-slug",
                    &[
                        ("id", id.to_string().into()),
                        ("other", other.to_string().into()),
                    ],
                )));
            }
            if !slugs.insert(slug) {
                return Err(selection_error(langs));
            }
        }
        Ok(())
    }

    async fn prepare_install_decision(
        &self,
        package: PluginPackage,
        request: &InstallRequest,
        enforce_plan: bool,
        langs: &Languages,
    ) -> Result<PreparedInstall, InstallError> {
        let manifest = &package.manifest;
        crate::plugin::install_entry::check_runtime(manifest, langs)?;
        // 摘要对不上就是变过了。计划只在请求带了才比：空计划是老调用方，和确认门槛一样只按摘要；
        // 带了计划的仍严格要求原样带回。`enforce_plan` 只由使用方的能力声明决定，批次里的提供方沿用使用方的值。
        if package.digest() != request.digest
            || (enforce_plan
                && !request.plan.is_empty()
                && self.dependency_plan(&package)? != request.plan)
        {
            return Err(InstallError::Changed(l10n::text(
                langs,
                "plugin-capability-plan-changed",
                &[],
            )));
        }
        if !matches!(request.scope, None | Some(PluginScope::All)) {
            return Err(InstallError::Unsupported(l10n::text(
                langs,
                "plugin-capability-choices-unsupported",
                &[],
            )));
        }
        if self.is_installed(&manifest.id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-already-installed",
                &[("id", manifest.id.to_string().into())],
            )));
        }
        let granted = crate::plugin::store::parse_permissions(&request.grant)
            .map_err(|error| InstallError::Invalid(error.to_string()))?;
        let enabled = crate::plugin::store::parse_enabled(
            &request.enabled_permissions,
            OpenedBy::Owner,
            crate::plugin::manager::now_secs(),
        )
        .map_err(|error| InstallError::Invalid(error.to_string()))?;
        crate::plugin::grant::check_permissions(manifest, &granted, &enabled, langs)?;
        crate::plugin::capabilities::unsupported_model_dependency(manifest, langs)?;
        self.check_requirements(manifest, langs).await?;
        self.check_egress(manifest, langs).await?;
        crate::plugin::render::dry_run(&package, langs)?;
        self.check_icon(&package, langs).await?;
        let effective =
            manifest.effective_permissions(&crate::plugin::store::permission_set(&enabled));
        self.gpu_grant(manifest, &effective, langs).await?;
        self.lan_grant(&package, request.lan_webui, langs).await?;
        self.resolve_devices(manifest, &effective, &request.devices, false, langs)?;
        let store = self.store.secrets();
        let secrets_before = store.load(&manifest.id)?;
        let mut secrets = secrets_before.clone().unwrap_or_default();
        crate::plugin::secret_input::check(manifest, &request.secrets, langs)?;
        for (name, value) in &request.secrets {
            crate::plugin::secret_input::store(
                manifest,
                &mut secrets,
                name,
                Some(value.expose()),
                langs,
            )?;
        }
        let targets = self.confirmed_install_targets(manifest)?;
        let changed_before = store.changed(&manifest.id)?;
        let settings = layer_settings(&package, &request.settings, langs)?;
        Ok(PreparedInstall {
            decision: InstallDecision {
                package,
                granted,
                enabled,
                lan_webui: request.lan_webui,
                targets: Some(targets),
                devices: request.devices.clone(),
            },
            inputs: request.secrets.clone(),
            settings,
            secrets_before,
            changed_before,
        })
    }

    pub(super) fn validate_connect_plugins(
        &self,
        manifest: &crate::plugin::manifest::Manifest,
        selected: &[String],
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let wanting = self.wanting_capability_consumers(manifest, langs)?;
        let mut seen = BTreeSet::new();
        if selected
            .iter()
            .any(|id| !seen.insert(id) || !wanting.iter().any(|consumer| &consumer.id == id))
        {
            return Err(selection_error(langs));
        }
        Ok(())
    }
}

fn selection_error(langs: &Languages) -> InstallError {
    InstallError::Invalid(l10n::text(
        langs,
        "plugin-capability-install-selection",
        &[],
    ))
}

/// 这一层包设置经 schema 初值化，未知字段如实报错；与 by_source 的商店安装同一套校验。
fn layer_settings(
    package: &PluginPackage,
    given: &wonton_protocol::SettingValues,
    langs: &Languages,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, InstallError> {
    match crate::plugin::settings::load(package, langs)? {
        Some(schema) => crate::plugin::settings::value::initial(&schema, given, langs)
            .map_err(|fields| crate::plugin::settings::value_error(fields, langs)),
        None if given.is_empty() => Ok(Default::default()),
        None => Err(crate::plugin::settings::value_error(
            given
                .keys()
                .map(|name| wonton_protocol::SettingsFieldError {
                    name: name.clone(),
                    kind: wonton_protocol::SettingsFieldProblemKind::UnknownField,
                    message: crate::l10n::text(langs, "plugin-settings-value-unknown-field", &[]),
                })
                .collect(),
            langs,
        )),
    }
}
