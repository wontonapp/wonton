//! 网页服务的健康门：显式健康路径或没有容器探针时，由核心直连网页入口。

use tokio::time::Instant;

use super::compose::DependCondition;
use super::health::{self, Checked, Readiness, Target, WebuiHealth};
use super::install_notes::note_dropped_capabilities;
use super::languages::Languages;
use super::manager::{Health, InstallError, PluginManager};
use super::manifest::Manifest;
use super::render::Deployment;
use crate::http::HttpClient;
use crate::l10n;

impl WebuiHealth {
    pub(super) fn applies(&self, has_container_check: bool) -> bool {
        !self.fallback || !has_container_check
    }

    pub(super) async fn probe(&self, http: &HttpClient) -> Result<(), String> {
        match super::upstreams::status(
            http,
            &self.endpoint,
            &self.path,
            super::upstreams::PROBE_TIMEOUT,
        )
        .await
        {
            Ok(200..=499) => Ok(()),
            Ok(code) => Err(format!("HTTP {code}")),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl PluginManager {
    pub(super) async fn webui_health(
        &self,
        manifest: &Manifest,
        service: &str,
    ) -> Result<Option<WebuiHealth>, InstallError> {
        let Some(webui) = manifest.webui.as_ref().filter(|web| web.service == service) else {
            return Ok(None);
        };
        let network = self.cfg.instance.network(&manifest.id.slug());
        let path = webui.health.as_ref().unwrap_or(&webui.path);
        let endpoint = self
            .webui_upstream(manifest, &network)
            .await?
            .ok_or_else(|| {
                InstallError::Unhealthy(probe_failure(
                    service,
                    path,
                    "no known address",
                    &self.owner_languages(),
                ))
            })?;
        Ok(Some(WebuiHealth {
            endpoint,
            path: path.clone(),
            fallback: webui.health.is_none(),
        }))
    }

    pub(super) async fn wait_ready_containers(
        &self,
        manifest: Option<&Manifest>,
        plan: &Deployment,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<Health, InstallError> {
        let mut healthy = Vec::new();
        for container in plan
            .containers
            .iter()
            .filter(|container| !container.one_shot)
        {
            let webui = match manifest {
                Some(manifest) => self.webui_health(manifest, &container.service).await?,
                None => None,
            };
            let readiness = Box::pin(health::wait_for(
                &self.engine,
                &self.http,
                &container.name,
                Target::Ready,
                webui.as_ref(),
                deadline,
                langs,
            ))
            .await
            .map_err(|error| note_dropped_capabilities(error, container, langs))?;
            healthy.push(readiness == Readiness::Healthy);
        }
        Ok(Health::of(healthy))
    }

    pub(super) async fn wait_dependencies(
        &self,
        manifest: Option<&Manifest>,
        deps: &[(String, DependCondition)],
        deadline: Instant,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let mut probe = None;
        if let Some(manifest) = manifest
            && let Some(webui) = &manifest.webui
        {
            let slug = manifest.id.slug();
            let name = if super::bot_web_registry::has_bot_web(manifest) {
                self.cfg
                    .instance
                    .bot_container(&slug, &webui.service, "default")
            } else {
                self.cfg.instance.container(&slug, &webui.service)
            };
            if deps.iter().any(|(dep, condition)| {
                dep == &name && *condition == DependCondition::ServiceHealthy
            }) && let Some(webui) = self.webui_health(manifest, &webui.service).await?
            {
                probe = Some((name, webui));
            }
        }
        health::wait_for_all(
            &self.engine,
            &self.http,
            deps,
            probe.as_ref().map(|(name, webui)| (name.as_str(), webui)),
            deadline,
            langs,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn check_webui_health(
        &self,
        manifest: &Manifest,
        service: &str,
        container: &str,
        checked: Checked,
        langs: &Languages,
    ) -> Checked {
        let Some(webui) = manifest.webui.as_ref().filter(|web| web.service == service) else {
            return checked;
        };
        if !matches!(
            checked,
            Checked::Healthy | Checked::Starting | Checked::Running
        ) || (webui.health.is_none() && checked != Checked::Running)
        {
            return checked;
        }
        match self.webui_health(manifest, service).await {
            Ok(Some(probe)) => match probe.probe(&self.http).await {
                Ok(()) => Checked::Healthy,
                Err(reason) => {
                    Checked::Failed(probe_failure(container, &probe.path, &reason, langs))
                }
            },
            Ok(None) => checked,
            Err(error) => Checked::Failed(error.to_string()),
        }
    }
}

fn probe_failure(container: &str, path: &str, reason: &str, langs: &Languages) -> String {
    l10n::text(
        langs,
        "plugin-install-container-health-probe-failed",
        &[
            ("container", container.into()),
            ("path", path.into()),
            ("reason", reason.into()),
        ],
    )
}
