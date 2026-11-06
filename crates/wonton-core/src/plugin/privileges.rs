//! 插件 Compose 高权限字段与清单权限的映射和兑现。

use std::collections::{BTreeMap, BTreeSet};

use bollard::models::{DeviceMapping, HostConfig};

use super::compose::{NetworkMode, Service, VolumeEntry, VolumeType};
use super::manifest::{Manifest, ManifestError, Permission, PluginKind, Sso};

pub const DOCKER_SOCKET: &str = "/var/run/docker.sock";

/// `cap_add` 的风险级别。名单外能力按最高级处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityRisk {
    High,
    VeryHigh,
    Highest,
}

/// 规范化并校验 Compose 的 capability 名字。
pub fn normalize_capability(name: &str) -> Option<String> {
    let name = name.trim().to_ascii_uppercase();
    (!name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'))
    .then_some(name)
}

pub fn capability_permission(name: &str) -> Permission {
    Permission::Capability {
        name: normalize_capability(name).unwrap_or_else(|| name.trim().to_ascii_uppercase()),
    }
}

/// `cap_add` 的风险级别；未列出的 Linux capability 一律按最高级。
pub fn capability_risk(name: &str) -> CapabilityRisk {
    match normalize_capability(name).as_deref() {
        Some(
            "NET_ADMIN" | "NET_RAW" | "NET_BIND_SERVICE" | "SYS_TIME" | "SYS_NICE" | "IPC_LOCK",
        ) => CapabilityRisk::VeryHigh,
        _ => CapabilityRisk::Highest,
    }
}

/// 权限对应的风险级别，供确认页和清单限制共用。
pub fn risk(permission: &Permission) -> Option<CapabilityRisk> {
    match permission {
        Permission::EgressProxy
        | Permission::NetworkLan
        | Permission::DevicesGpu
        | Permission::Device { .. } => Some(CapabilityRisk::High),
        Permission::Capability { name } => Some(capability_risk(name)),
        Permission::NetworkHost => Some(CapabilityRisk::VeryHigh),
        Permission::HostPid
        | Permission::HostIpc
        | Permission::SecurityOpt
        | Permission::Privileged
        | Permission::DockerSocket => Some(CapabilityRisk::Highest),
        _ => None,
    }
}

/// 固定设备不扩大宿主路径的允许范围；串口和 USB 由后续的设备选择流程处理。
pub fn is_fixed_device(path: &str) -> bool {
    matches!(path, "/dev/net/tun" | "/dev/fuse")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    pub service: String,
    pub message: String,
}

impl ServiceError {
    fn new(service: &str, message: impl Into<String>) -> Self {
        Self {
            service: service.to_owned(),
            message: message.into(),
        }
    }
}

/// 清单级别的权限约束。
pub fn validate_manifest(manifest: &Manifest) -> Result<(), ManifestError> {
    let restricted = manifest.kind == PluginKind::Agent
        || manifest.permissions.contains(&Permission::MatrixBot)
        || manifest.bots.is_some();
    if restricted
        && let Some(permission) = manifest.permissions.iter().find(|permission| {
            matches!(
                risk(permission),
                Some(CapabilityRisk::VeryHigh | CapabilityRisk::Highest)
            )
        })
    {
        return Err(ManifestError::Invalid(format!(
            "Agent 或 Bot 插件不能申请很高或最高级权限 {permission}"
        )));
    }
    if let Some(permission) = manifest
        .permissions
        .iter()
        .find_map(|permission| match permission {
            Permission::Device { path } if !is_fixed_device(path) => Some(path),
            _ => None,
        })
    {
        return Err(ManifestError::Invalid(format!(
            "固定设备权限只支持 /dev/net/tun 和 /dev/fuse，不能申请 {permission}"
        )));
    }
    Ok(())
}

#[derive(Default)]
struct UsedPrivileges {
    caps: BTreeSet<String>,
    devices: BTreeSet<String>,
    host: bool,
    pid: bool,
    ipc: bool,
    security: bool,
    privileged: bool,
    socket: bool,
}

/// Compose 字段与清单权限必须逐项对应；这是安装、预览和 `plugin check` 共用的校验。
pub fn validate_services(
    manifest: &Manifest,
    services: &BTreeMap<String, Service>,
) -> Result<(), ServiceError> {
    let mut used = UsedPrivileges::default();
    for (name, service) in services {
        validate_service(manifest, name, service, &mut used)?;
    }
    validate_used_permissions(manifest, &used)
}

fn validate_service(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
) -> Result<(), ServiceError> {
    validate_host_network(manifest, name, service, used)?;
    validate_capabilities(manifest, name, service, used)?;
    validate_devices(manifest, name, service, used)?;
    validate_namespaces(manifest, name, service, used)?;
    validate_socket(manifest, name, service, used)
}

fn validate_host_network(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
) -> Result<(), ServiceError> {
    if service.network_mode != Some(NetworkMode::Host) {
        return Ok(());
    }
    used.host = true;
    require(
        manifest,
        Permission::NetworkHost,
        name,
        "network_mode: host",
    )?;
    if manifest
        .lan
        .as_ref()
        .is_some_and(|lan| lan.ports.iter().any(|port| port.service == name))
    {
        return Err(ServiceError::new(
            name,
            "宿主网络服务不能同时声明 lan.ports；端口由应用直接监听宿主网络",
        ));
    }
    if manifest
        .webui
        .as_ref()
        .is_some_and(|webui| webui.service == name)
    {
        let webui = manifest
            .webui
            .as_ref()
            .expect("上面的条件已确认 webui 存在");
        if webui.sso == Sso::Proxy {
            return Err(ServiceError::new(
                name,
                "宿主网络里的 webui 不能使用 sso: proxy，请改用 oidc 或 none",
            ));
        }
        if service
            .healthcheck
            .as_ref()
            .is_none_or(|check| check.disable || check.test.is_none())
        {
            return Err(ServiceError::new(
                name,
                "宿主网络里的 webui 服务必须写有效的 healthcheck",
            ));
        }
    }
    Ok(())
}

fn validate_capabilities(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
) -> Result<(), ServiceError> {
    for cap in &service.cap_add {
        let capability = normalize_capability(cap)
            .ok_or_else(|| ServiceError::new(name, format!("cap_add 里的能力 `{cap}` 不合法")))?;
        require(
            manifest,
            Permission::Capability {
                name: capability.clone(),
            },
            name,
            &format!("cap_add: {cap}"),
        )?;
        used.caps.insert(capability);
    }
    Ok(())
}

fn validate_devices(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
) -> Result<(), ServiceError> {
    for device in &service.devices {
        let (source, target, permissions) = device
            .paths()
            .map_err(|message| ServiceError::new(name, message))?;
        if !is_fixed_device(&source) {
            return Err(ServiceError::new(
                name,
                format!("设备 `{source}` 不是允许的固定设备"),
            ));
        }
        if !target.starts_with('/') {
            return Err(ServiceError::new(
                name,
                format!("设备目标 `{target}` 必须是绝对路径"),
            ));
        }
        if permissions.is_empty()
            || permissions
                .bytes()
                .any(|byte| !matches!(byte, b'r' | b'w' | b'm'))
        {
            return Err(ServiceError::new(
                name,
                format!("设备 `{source}` 的权限 `{permissions}` 不合法"),
            ));
        }
        require(
            manifest,
            Permission::Device {
                path: source.clone(),
            },
            name,
            &format!("devices: {source}"),
        )?;
        used.devices.insert(source);
    }
    Ok(())
}

fn validate_namespaces(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
) -> Result<(), ServiceError> {
    if let Some(pid) = &service.pid {
        if pid != "host" {
            return Err(ServiceError::new(name, "pid 只支持 host"));
        }
        require(manifest, Permission::HostPid, name, "pid: host")?;
        used.pid = true;
    }
    if let Some(ipc) = &service.ipc {
        if ipc != "host" {
            return Err(ServiceError::new(name, "ipc 只支持 host"));
        }
        require(manifest, Permission::HostIpc, name, "ipc: host")?;
        used.ipc = true;
    }
    if !service.security_opt.is_empty() {
        require(manifest, Permission::SecurityOpt, name, "security_opt")?;
        used.security = true;
    }
    if service.privileged == Some(true) {
        require(manifest, Permission::Privileged, name, "privileged: true")?;
        used.privileged = true;
    }
    Ok(())
}

fn validate_socket(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
) -> Result<(), ServiceError> {
    for entry in &service.volumes {
        let Some((source, target, read_only)) = volume_mount(entry) else {
            continue;
        };
        if source == DOCKER_SOCKET || target == DOCKER_SOCKET {
            if source != DOCKER_SOCKET || target != DOCKER_SOCKET || read_only {
                return Err(ServiceError::new(
                    name,
                    "Docker socket 只能以 /var/run/docker.sock:/var/run/docker.sock 可写挂载",
                ));
            }
            require(manifest, Permission::DockerSocket, name, "docker.sock 挂载")?;
            used.socket = true;
        }
    }
    Ok(())
}

fn validate_used_permissions(
    manifest: &Manifest,
    used: &UsedPrivileges,
) -> Result<(), ServiceError> {
    check_declared(
        manifest,
        Permission::NetworkHost,
        used.host,
        "network_mode: host",
    )?;
    check_declared(manifest, Permission::HostPid, used.pid, "pid: host")?;
    check_declared(manifest, Permission::HostIpc, used.ipc, "ipc: host")?;
    check_declared(
        manifest,
        Permission::SecurityOpt,
        used.security,
        "security_opt",
    )?;
    check_declared(
        manifest,
        Permission::Privileged,
        used.privileged,
        "privileged: true",
    )?;
    check_declared(
        manifest,
        Permission::DockerSocket,
        used.socket,
        "docker.sock 挂载",
    )?;
    for permission in &manifest.permissions {
        match permission {
            Permission::Capability { name } if !used.caps.contains(name) => {
                return Err(ServiceError::new(
                    "*",
                    format!("权限 {permission} 没有对应的 cap_add 字段"),
                ));
            }
            Permission::Device { path } if !used.devices.contains(path) => {
                return Err(ServiceError::new(
                    "*",
                    format!("权限 {permission} 没有对应的 devices 字段"),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn require(
    manifest: &Manifest,
    permission: Permission,
    service: &str,
    field: &str,
) -> Result<(), ServiceError> {
    if manifest.permissions.contains(&permission) {
        Ok(())
    } else {
        Err(ServiceError::new(
            service,
            format!("{field} 需要清单权限 {permission}"),
        ))
    }
}

fn check_declared(
    manifest: &Manifest,
    permission: Permission,
    used: bool,
    field: &str,
) -> Result<(), ServiceError> {
    if manifest.permissions.contains(&permission) && !used {
        Err(ServiceError::new(
            "*",
            format!("清单权限 {permission} 没有对应的 {field} 字段"),
        ))
    } else {
        Ok(())
    }
}

fn volume_mount(entry: &VolumeEntry) -> Option<(String, String, bool)> {
    match entry {
        VolumeEntry::Short(text) => {
            let parts: Vec<&str> = text.split(':').collect();
            let (source, target, mode) = match parts.as_slice() {
                [source, target] => (*source, *target, ""),
                [source, target, mode] => (*source, *target, *mode),
                _ => return None,
            };
            Some((
                source.to_owned(),
                target.to_owned(),
                mode.split(',').any(|item| item == "ro"),
            ))
        }
        VolumeEntry::Long(volume) if volume.kind == VolumeType::Bind => Some((
            volume.source.clone()?,
            volume.target.clone(),
            volume.read_only,
        )),
        _ => None,
    }
}

/// 把已校验的 Compose 高权限字段写入 Docker HostConfig。
pub fn apply(service: &Service, host: &mut HostConfig) -> Result<(), String> {
    if !service.cap_add.is_empty() {
        host.cap_add = Some(
            service
                .cap_add
                .iter()
                .map(|cap| {
                    normalize_capability(cap)
                        .ok_or_else(|| format!("cap_add 里的能力 `{cap}` 不合法"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    if service.network_mode == Some(NetworkMode::Host)
        && !service
            .cap_add
            .iter()
            .any(|cap| normalize_capability(cap).as_deref() == Some("NET_RAW"))
    {
        host.cap_drop = Some(vec!["NET_RAW".to_owned()]);
    }
    if service.pid.as_deref() == Some("host") {
        host.pid_mode = Some("host".to_owned());
    }
    if service.ipc.as_deref() == Some("host") {
        host.ipc_mode = Some("host".to_owned());
    }
    if !service.security_opt.is_empty() {
        host.security_opt = Some(service.security_opt.clone());
    }
    if service.privileged == Some(true) {
        host.privileged = Some(true);
    }
    for device in &service.devices {
        let (source, target, permissions) = device.paths()?;
        host.devices
            .get_or_insert_with(Vec::new)
            .push(DeviceMapping {
                path_on_host: Some(source),
                path_in_container: Some(target),
                cgroup_permissions: Some(permissions),
            });
    }
    Ok(())
}
