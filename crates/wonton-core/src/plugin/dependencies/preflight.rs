use std::collections::{BTreeMap, BTreeSet};

use wonton_protocol::{InstallRequest, PluginScope, SecretValues};

use super::{InstallError, Languages, PluginManager, generic, service};
use crate::l10n;
use crate::plugin::installation::InstallDecision;
use crate::plugin::manifest::PluginKind;
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
        langs: &Languages,
    ) -> Result<Vec<PreparedInstall>, InstallError> {
        let preview = self.dependency_preview(&package.manifest, langs).await?;
        let mut consumer = self
            .prepare_install_decision(package, &request, langs)
            .await?;
        let mut ids = BTreeSet::from([consumer.decision.package.manifest.id.clone()]);
        let candidates = if request.install_together.is_empty() {
            Vec::new()
        } else {
            self.dependency_candidates(langs).await?
        };
        let mut providers = Vec::with_capacity(request.install_together.len());
        for requested in &request.install_together {
            let inner = &requested.request;
            if !inner.install_together.is_empty() || !inner.connect_plugins.is_empty() {
                return Err(selection_error(langs));
            }
            let candidate = candidates
                .iter()
                .find(|candidate| {
                    candidate.listed.manifest.id.as_str() == requested.id
                        && preview.missing_requires.iter().any(|missing| {
                            missing.providers.iter().any(|provider| {
                                provider.id == requested.id && provider.source == candidate.source()
                            })
                        })
                })
                .ok_or_else(|| selection_error(langs))?;
            let package = self.fetch_source(&inner.source).await?;
            if package.digest() != inner.digest {
                return Err(InstallError::Changed(l10n::text(
                    langs,
                    "plugin-capability-plan-changed",
                    &[],
                )));
            }
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
            if package.manifest.id.as_str() != requested.id
                || !ids.insert(package.manifest.id.clone())
            {
                return Err(selection_error(langs));
            }
            if !package.manifest.requires.is_empty() {
                return Err(selection_error(langs));
            }
            providers.push(self.prepare_install_decision(package, inner, langs).await?);
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
        langs: &Languages,
    ) -> Result<PreparedInstall, InstallError> {
        let manifest = &package.manifest;
        crate::plugin::install_entry::check_runtime(manifest, langs)?;
        if package.digest() != request.digest || self.dependency_plan(&package)? != request.plan {
            return Err(InstallError::Changed(l10n::text(
                langs,
                "plugin-capability-plan-changed",
                &[],
            )));
        }
        if manifest.kind == PluginKind::System {
            return Err(InstallError::Unsupported(l10n::text(
                langs,
                "install-system-preinstalled",
                &[],
            )));
        }
        if !request.devices.is_empty() || !matches!(request.scope, None | Some(PluginScope::All)) {
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
        let store = self.store.secrets();
        let secrets_before = store.load(&manifest.id)?;
        let mut secrets = secrets_before.clone().unwrap_or_default();
        crate::plugin::secret_input::check(
            manifest,
            &request.secrets,
            &|name| secrets.contains_key(name),
            langs,
        )?;
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
        // 与 by_source 的商店安装同一套校验：包设置经 schema 初值化，未知字段如实报错。
        let settings = match super::settings::load(&package, langs)? {
            Some(schema) => super::settings::value::initial(&schema, &request.settings, langs)
                .map_err(|fields| super::settings::value_error(fields, langs))?,
            None if request.settings.is_empty() => Default::default(),
            None => {
                let fields = request
                    .settings
                    .keys()
                    .map(|name| wonton_protocol::SettingsFieldError {
                        name: name.clone(),
                        kind: wonton_protocol::SettingsFieldProblemKind::UnknownField,
                        message: crate::l10n::text(langs, "plugin-settings-value-unknown-field", &[]),
                    })
                    .collect();
                return Err(super::settings::value_error(fields, langs));
            }
        };
        Ok(PreparedInstall {
            decision: InstallDecision {
                package,
                granted,
                enabled,
                lan_webui: request.lan_webui,
                targets: Some(targets),
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
