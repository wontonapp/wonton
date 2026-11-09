//! `provides.<能力>` 的服务和 compose 的对照（ADR-0053）：`check_manifest_references` 的通用能力部分。

use serde_json::Value;

use super::{PackageError, PluginPackage};
use crate::l10n::text;
use crate::plugin::languages::Languages;

impl PluginPackage {
    /// 提供方的服务和跑凭据命令的服务都要在 compose 里，且不能是 `network_mode: none` 的——核心要
    /// 转发进去、在里面签发凭据。
    pub(super) fn check_capability_services(&self, langs: &Languages) -> Result<(), PackageError> {
        for (name, provider) in self.manifest.provides.iter().flat_map(|p| &p.capabilities) {
            let mut refs = vec![(
                format!("provides.{name}.service"),
                provider.service.as_str(),
            )];
            if let Some(service) = provider
                .credentials
                .as_ref()
                .and_then(|credentials| credentials.service.as_ref())
            {
                refs.push((
                    format!("provides.{name}.credentials.service"),
                    service.as_str(),
                ));
            }
            for (field, service) in refs {
                if !self.services.contains_key(service) {
                    return Err(PackageError::Service {
                        service: service.to_owned(),
                        message: text(
                            langs,
                            "plugin-capability-service-unknown",
                            &[
                                ("field", field.as_str().into()),
                                ("service", service.into()),
                            ],
                        ),
                    });
                }
                if self.services[service]
                    .as_object()
                    .and_then(|service| service.get("network_mode"))
                    .and_then(Value::as_str)
                    == Some("none")
                {
                    return Err(PackageError::Service {
                        service: service.to_owned(),
                        message: text(
                            langs,
                            "plugin-capability-service-not-forwardable",
                            &[("field", field.as_str().into())],
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}
