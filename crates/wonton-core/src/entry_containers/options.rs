//! 判断运行中的入口容器是否仍符合期望的安全选项。

use bollard::models::{ContainerCreateBody, ContainerInspectResponse, PortBinding};
use std::collections::HashMap;

pub(super) fn same_options(info: &ContainerInspectResponse, body: &ContainerCreateBody) -> bool {
    let (Some(config), Some(host)) = (info.config.as_ref(), info.host_config.as_ref()) else {
        return false;
    };
    if config.user != body.user || host.dns != body.host_config.as_ref().and_then(|h| h.dns.clone())
    {
        return false;
    }
    let wanted_host = body.host_config.as_ref();
    let binds = |list: Option<&Vec<String>>| list.cloned().unwrap_or_default();
    if binds(host.binds.as_ref()) != binds(wanted_host.and_then(|h| h.binds.as_ref())) {
        return false;
    }
    let ports =
        |map: Option<&HashMap<String, Option<Vec<PortBinding>>>>| map.cloned().unwrap_or_default();
    if ports(host.port_bindings.as_ref())
        != ports(wanted_host.and_then(|h| h.port_bindings.as_ref()))
    {
        return false;
    }
    let have: std::collections::BTreeSet<&str> = config
        .env
        .as_ref()
        .map(|env| env.iter().map(String::as_str).collect())
        .unwrap_or_default();
    body.env
        .as_ref()
        .is_none_or(|env| env.iter().all(|item| have.contains(item.as_str())))
}
