//! 同插件服务共用网络命名空间的声明边界与重建范围。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::compose::{DependCondition, NetworkMode, Service};
use super::manifest::Manifest;
use super::package::PackageError;
use super::render::Deployment;

pub(super) fn add_dependency(dependencies: &mut Vec<(String, DependCondition)>, target: &str) {
    if !dependencies.iter().any(|(name, condition)| {
        name == target && *condition != DependCondition::ServiceCompletedSuccessfully
    }) {
        dependencies.push((target.to_owned(), DependCondition::ServiceStarted));
    }
}

pub(super) fn validate_fields(name: &str, fields: &Map<String, Value>) -> Result<(), PackageError> {
    if !fields
        .get("network_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode.starts_with("service:"))
    {
        return Ok(());
    }
    for field in [
        "networks",
        "ports",
        "hostname",
        "domainname",
        "dns",
        "dns_search",
        "extra_hosts",
        "mac_address",
        "expose",
    ] {
        if fields.contains_key(field) {
            return Err(invalid(
                name,
                format!("共用网络命名空间的服务不能写 `{field}`；请把它写在目标服务上"),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate(
    manifest: &Manifest,
    services: &BTreeMap<String, Service>,
) -> Result<(), PackageError> {
    let is_bot = |name: &str| {
        manifest
            .bots
            .as_ref()
            .is_some_and(|bots| bots.services.iter().any(|s| s == name))
    };
    for (name, service) in services {
        let Some(NetworkMode::Service(target)) = &service.network_mode else {
            continue;
        };
        let destination = services
            .get(target)
            .filter(|_| target != name)
            .ok_or_else(|| {
                invalid(
                    name,
                    format!("network_mode 必须指向本插件里的另一个服务，目标 `{target}` 不合法"),
                )
            })?;
        if destination.network_mode.is_some() {
            return Err(invalid(
                name,
                format!(
                    "network_mode 的目标服务 `{target}` 不能再写 network_mode；不能链式共用或借用宿主网络"
                ),
            ));
        }
        if is_bot(name) != is_bot(target) {
            return Err(invalid(
                name,
                format!("共用服务和目标 `{target}` 必须都在 bots.services 里，或都不在其中"),
            ));
        }
        let entries = [
            ("webui.service", manifest.webui.as_ref().map(|w| &w.service)),
            ("mcp.service", manifest.mcp.as_ref().map(|m| &m.service)),
        ];
        let lan = manifest
            .lan
            .iter()
            .flat_map(|lan| &lan.ports)
            .map(|port| ("lan.ports", Some(&port.service)));
        if let Some((field, _)) = entries
            .into_iter()
            .chain(lan)
            .find(|(_, entry)| *entry == Some(name))
        {
            return Err(invalid(
                name,
                format!("{field} 不能指向共用网络命名空间的服务；请改成目标服务 `{target}`"),
            ));
        }
    }
    Ok(())
}

fn invalid(service: &str, message: String) -> PackageError {
    PackageError::Service {
        service: service.to_owned(),
        message,
    }
}

impl Deployment {
    /// HTTP 的连接地址属于网络命名空间所有者；命令仍在清单声明的服务里执行。
    /// 只接受唯一的全插件服务，不能把缺失或每 Bot 的容器偷偷换成另一份。
    pub(super) fn clients_http_network_service(&self, declared: &str) -> Option<&str> {
        let mut declared_plans = self
            .containers
            .iter()
            .filter(|plan| plan.service == declared);
        let plan = declared_plans.next()?;
        if declared_plans.next().is_some() || plan.bot.is_some() {
            return None;
        }
        let mode = plan.body.host_config.as_ref()?.network_mode.as_deref();
        if mode == Some(self.network.as_str()) {
            return Some(&plan.service);
        }
        let target = mode?.strip_prefix("container:")?;
        let mut targets = self.containers.iter().filter(|plan| plan.name == target);
        let target = targets.next()?;
        if targets.next().is_some()
            || target.bot.is_some()
            || target.body.host_config.as_ref()?.network_mode.as_deref()
                != Some(self.network.as_str())
        {
            return None;
        }
        Some(&target.service)
    }

    /// Docker 的 container: 绑定容器 ID，目标换掉时必须一起换掉共用者。
    pub(super) fn retain_services(&mut self, services: &BTreeSet<String>) {
        let targets: BTreeSet<String> = self
            .containers
            .iter()
            .filter(|container| services.contains(container.service.as_str()))
            .map(|container| format!("container:{}", container.name))
            .collect();
        self.containers.retain(|container| {
            services.contains(container.service.as_str())
                || container
                    .body
                    .host_config
                    .as_ref()
                    .and_then(|host| host.network_mode.as_ref())
                    .is_some_and(|mode| targets.contains(mode))
        });
    }
}
