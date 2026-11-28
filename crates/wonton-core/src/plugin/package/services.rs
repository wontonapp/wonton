//! 插件包里的服务：名字、依赖、一次性与常驻服务，以及插值后按 Compose 子集解析。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::super::compose::{Service, strip_extensions};
use super::super::interpolate::{UsedVars, Vars, interpolate_value};
use super::super::languages::Languages;
use super::errors::{interpolate_error, service_error};
use super::{PackageError, PluginPackage, pathed};

pub(in crate::plugin) type ServiceDependencies<'a> =
    BTreeMap<&'a str, Vec<(String, super::super::compose::DependCondition)>>;

impl PluginPackage {
    /// 插值后按 Compose 子集解析所有服务。`vars` 给出每个服务用的变量表：Bot 服务比全插件一份的服务
    /// 多几个变量。
    pub fn services<'v>(
        &self,
        vars: impl Fn(&str) -> &'v Vars,
        used: &mut BTreeMap<String, UsedVars>,
        langs: &Languages,
    ) -> Result<BTreeMap<String, Service>, PackageError> {
        let services: BTreeMap<String, Service> = self
            .services
            .iter()
            .map(|(name, _)| {
                let mut variables = UsedVars::new();
                let service = self.service(name, vars(name), &mut variables, langs)?;
                if !variables.is_empty() {
                    used.insert(name.clone(), variables);
                }
                Ok((name.clone(), service))
            })
            .collect::<Result<_, PackageError>>()?;
        super::super::network_mode::validate(&self.manifest, &services)?;
        Ok(services)
    }

    pub(in crate::plugin) fn service(
        &self,
        name: &str,
        vars: &Vars,
        used: &mut UsedVars,
        langs: &Languages,
    ) -> Result<Service, PackageError> {
        let mut value = self.services[name].clone();
        strip_extensions(&mut value);
        interpolate_value(&mut value, vars, used)
            .map_err(|source| interpolate_error(langs, name.to_owned(), source))?;
        serde_path_to_error::deserialize(value).map_err(|e| service_error(langs, name, pathed(&e)))
    }

    /// 服务的 `environment` 的原文（没插值）：引用了哪些变量、写了什么字面值。
    pub fn raw_environment(&self, service: &str) -> String {
        let environment = self.services.get(service).map(|raw| &raw["environment"]);
        environment.map(ToString::to_string).unwrap_or_default()
    }

    pub fn service_names(&self) -> impl Iterator<Item = &str> {
        self.services.keys().map(String::as_str)
    }

    /// 启动现存容器只需要依赖条件，不重新插值环境或改写容器配置。
    pub(in crate::plugin) fn service_dependencies(
        &self,
        langs: &Languages,
    ) -> Result<ServiceDependencies<'_>, PackageError> {
        self.services
            .iter()
            .map(|(name, service)| {
                let dependencies = service
                    .get("depends_on")
                    .map(|value| {
                        serde_path_to_error::deserialize::<_, super::super::compose::DependsOn>(
                            value.clone(),
                        )
                    })
                    .transpose()
                    .map_err(|e| service_error(langs, name, pathed(&e)))?;
                let mut dependencies = dependencies.map(|deps| deps.entries()).unwrap_or_default();
                if let Some(target) = service
                    .get("network_mode")
                    .and_then(Value::as_str)
                    .and_then(|mode| mode.strip_prefix("service:"))
                {
                    super::super::network_mode::add_dependency(&mut dependencies, target);
                }
                Ok((name.as_str(), dependencies))
            })
            .collect()
    }

    pub fn named_volumes(&self) -> &BTreeSet<String> {
        &self.volumes
    }

    /// 一次性服务：写了 `restart: "no"` 的服务，和被其他服务以 `condition: service_completed_successfully`
    /// 依赖的服务（后者没写 `restart: "no"` 时渲染报错）。
    pub fn one_shot_services(&self) -> BTreeSet<&str> {
        let completed = Value::String("service_completed_successfully".to_owned());
        let no_restart = Value::String("no".to_owned());
        let depended_on = self
            .services
            .values()
            .filter_map(|service| service.get("depends_on")?.as_object())
            .flat_map(|deps| deps.iter())
            .filter(|(_, spec)| spec.get("condition") == Some(&completed))
            .filter_map(|(dep, _)| self.services.get_key_value(dep).map(|(k, _)| k.as_str()));
        self.services
            .iter()
            .filter(|(_, service)| service.get("restart") == Some(&no_restart))
            .map(|(name, _)| name.as_str())
            .chain(depended_on)
            .collect()
    }

    /// 长期运行的服务，也就是部署完成时全部能用的那些（[`Self::one_shot_services`] 之外）。
    pub fn resident_services(&self) -> BTreeSet<&str> {
        let one_shot = self.one_shot_services();
        self.service_names()
            .filter(|service| !one_shot.contains(service))
            .collect()
    }

    /// compose 里有没有这个服务。
    pub fn has_service(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }

    /// 有服务引用了 `WONTON_EXTERNAL_URL`：入口开关变了要重建它们（ADR-0089 第 5 节）。
    pub(in crate::plugin) fn references_external_url(&self) -> bool {
        super::super::manifest_clients::references_external_url(&self.services)
    }
}
