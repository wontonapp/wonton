//! 插件 Compose 高权限字段与清单权限的映射和兑现。

use std::collections::{BTreeMap, BTreeSet};

use bollard::models::{DeviceMapping, HostConfig};

use super::compose::{NetworkMode, Service, VolumeEntry, VolumeType};
use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Permission, PluginKind, Sso};
use crate::l10n::text;

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

/// `security_opt` 的一条写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityOpt {
    /// `no-new-privileges`：`true` 等于核心默认的加固，`false` 退回 Docker 默认。
    NoNewPrivileges(bool),
    /// 其他写法，要 `security.opt` 权限并原样交给 Docker。
    Other,
}

/// 按 Docker 的解析规则分类：名字大小写敏感，`:` 和 `=` 都是分隔符，值按 ParseBool 收。
pub fn parse_security_opt(opt: &str, langs: &Languages) -> Result<SecurityOpt, String> {
    let (name, value) = match opt.split_once([':', '=']) {
        Some((name, value)) => (name, Some(value)),
        None => (opt, None),
    };
    if name != "no-new-privileges" {
        return Ok(SecurityOpt::Other);
    }
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("true" | "t" | "1") => Ok(SecurityOpt::NoNewPrivileges(true)),
        Some("false" | "f" | "0") => Ok(SecurityOpt::NoNewPrivileges(false)),
        Some(_) => Err(text(
            langs,
            "plugin-install-priv-security-opt-invalid",
            &[("opt", opt.into())],
        )),
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

/// 可以声明成可选、默认关的权限（ADR-0057）。判据：关着时整段字段能拿掉，插件还能运行。
pub fn may_be_optional(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::DockerSocket
            | Permission::DevicesGpu
            | Permission::DevicesSerial
            | Permission::DevicesUsb
    )
}

/// 开着时注入给插件的变量名（ADR-0057）。注入给这个插件的所有服务，关着时不定义。
pub fn permission_var(permission: &Permission) -> Option<&'static str> {
    match permission {
        Permission::DockerSocket => Some("WONTON_PERMISSION_DOCKER_SOCKET"),
        Permission::DevicesGpu => Some("WONTON_PERMISSION_DEVICES_GPU"),
        Permission::DevicesSerial => Some("WONTON_PERMISSION_DEVICES_SERIAL"),
        Permission::DevicesUsb => Some("WONTON_PERMISSION_DEVICES_USB"),
        _ => None,
    }
}

/// `optional_permissions` 的键和 `off_loss`：在允许名单里、不和必需的重复、每种语言都写了。
pub fn validate_optional(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    for (permission, optional) in &manifest.optional_permissions {
        if !may_be_optional(permission) {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-install-optional-not-allowed",
                &[("permission", permission.to_string().into())],
            )));
        }
        if manifest.permissions.contains(permission) {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-install-optional-declared-twice",
                &[("permission", permission.to_string().into())],
            )));
        }
        let complete = ["zh", "en"].iter().all(|lang| {
            optional
                .off_loss
                .get(*lang)
                .is_some_and(|text| !text.trim().is_empty())
        });
        if !complete {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-install-optional-off-loss",
                &[("permission", permission.to_string().into())],
            )));
        }
    }
    Ok(())
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
pub fn validate_manifest(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let restricted = manifest.kind == PluginKind::Agent
        || manifest.permissions.contains(&Permission::MatrixBot)
        || manifest.bots.is_some();
    if restricted
        && let Some(permission) = manifest.all_permissions().find(|permission| {
            matches!(
                risk(permission),
                Some(CapabilityRisk::VeryHigh | CapabilityRisk::Highest)
            )
        })
    {
        return Err(ManifestError::Invalid(text(
            langs,
            "plugin-install-priv-agent-high-risk",
            &[("permission", permission.to_string().into())],
        )));
    }
    if let Some(permission) = manifest
        .all_permissions()
        .find_map(|permission| match permission {
            Permission::Device { path } if !is_fixed_device(path) => Some(path),
            _ => None,
        })
    {
        return Err(ManifestError::Invalid(text(
            langs,
            "plugin-install-priv-fixed-device",
            &[("permission", permission.into())],
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
    langs: &Languages,
) -> Result<(), ServiceError> {
    let mut used = UsedPrivileges::default();
    for (name, service) in services {
        validate_service(manifest, name, service, &mut used, langs)?;
    }
    validate_used_permissions(manifest, &used, langs)
}

fn validate_service(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
    langs: &Languages,
) -> Result<(), ServiceError> {
    validate_host_network(manifest, name, service, used, langs)?;
    validate_capabilities(manifest, name, service, used, langs)?;
    validate_cap_drop(name, service, langs)?;
    validate_devices(manifest, name, service, used, langs)?;
    validate_namespaces(manifest, name, service, used, langs)?;
    validate_socket(manifest, name, service, used, langs)
}

fn validate_host_network(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
    langs: &Languages,
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
        langs,
    )?;
    if manifest.permissions.contains(&Permission::Notify) {
        // 发信按来源网段认插件（ADR-0047）；宿主网络的信从网桥地址来，认不出来源会被丢，
        // 所以装时就拒（ADR-0056），失败即停。
        return Err(ServiceError::new(
            name,
            text(langs, "plugin-install-priv-host-network-mail", &[]),
        ));
    }
    if manifest
        .lan
        .as_ref()
        .is_some_and(|lan| lan.ports.iter().any(|port| port.service == name))
    {
        return Err(ServiceError::new(
            name,
            text(langs, "plugin-install-priv-host-network-ports", &[]),
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
                text(langs, "plugin-install-priv-host-network-sso", &[]),
            ));
        }
        // `webui.health` 时核心自己探，不用 compose 里的 healthcheck。
        if webui.health.is_none()
            && service
                .healthcheck
                .as_ref()
                .is_none_or(|check| check.disable || check.test.is_none())
        {
            return Err(ServiceError::new(
                name,
                text(langs, "plugin-install-priv-host-network-healthcheck", &[]),
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
    langs: &Languages,
) -> Result<(), ServiceError> {
    for cap in &service.cap_add {
        let capability = normalize_capability(cap).ok_or_else(|| {
            ServiceError::new(
                name,
                text(
                    langs,
                    "plugin-install-priv-cap-invalid",
                    &[("cap", cap.into())],
                ),
            )
        })?;
        require(
            manifest,
            Permission::Capability {
                name: capability.clone(),
            },
            name,
            &format!("cap_add: {cap}"),
            langs,
        )?;
        used.caps.insert(capability);
    }
    Ok(())
}

/// `cap_drop` 只减不加，不对应任何权限；值和 `cap_add` 一样规范化，`ALL` 也收。
fn validate_cap_drop(name: &str, service: &Service, langs: &Languages) -> Result<(), ServiceError> {
    for cap in &service.cap_drop {
        if normalize_capability(cap).is_none() {
            return Err(ServiceError::new(
                name,
                text(
                    langs,
                    "plugin-install-priv-cap-drop-invalid",
                    &[("cap", cap.into())],
                ),
            ));
        }
    }
    Ok(())
}

fn validate_devices(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
    langs: &Languages,
) -> Result<(), ServiceError> {
    for device in &service.devices {
        let (source, target, permissions) = device
            .paths(langs)
            .map_err(|message| ServiceError::new(name, message))?;
        if !is_fixed_device(&source) {
            return Err(ServiceError::new(
                name,
                text(
                    langs,
                    "plugin-install-priv-device-not-allowed",
                    &[("source", source.into())],
                ),
            ));
        }
        if !target.starts_with('/') {
            return Err(ServiceError::new(
                name,
                text(
                    langs,
                    "plugin-install-priv-device-absolute",
                    &[("target", target.into())],
                ),
            ));
        }
        if permissions.is_empty()
            || permissions
                .bytes()
                .any(|byte| !matches!(byte, b'r' | b'w' | b'm'))
        {
            return Err(ServiceError::new(
                name,
                text(
                    langs,
                    "plugin-install-priv-device-mode",
                    &[
                        ("source", source.into()),
                        ("permissions", permissions.into()),
                    ],
                ),
            ));
        }
        require(
            manifest,
            Permission::Device {
                path: source.clone(),
            },
            name,
            &format!("devices: {source}"),
            langs,
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
    langs: &Languages,
) -> Result<(), ServiceError> {
    if let Some(pid) = &service.pid {
        if pid != "host" {
            return Err(ServiceError::new(
                name,
                text(langs, "plugin-install-priv-pid-host-only", &[]),
            ));
        }
        require(manifest, Permission::HostPid, name, "pid: host", langs)?;
        used.pid = true;
    }
    if let Some(ipc) = &service.ipc {
        if ipc != "host" {
            return Err(ServiceError::new(
                name,
                text(langs, "plugin-install-priv-ipc-host-only", &[]),
            ));
        }
        require(manifest, Permission::HostIpc, name, "ipc: host", langs)?;
        used.ipc = true;
    }
    // no-new-privileges 的写法等于默认加固或退回 Docker 默认，都不是权限；其余写法照 ADR-0036 要 security.opt。
    let mut gated = false;
    for opt in &service.security_opt {
        match parse_security_opt(opt, langs).map_err(|message| ServiceError::new(name, message))? {
            SecurityOpt::NoNewPrivileges(_) => {}
            SecurityOpt::Other => gated = true,
        }
    }
    if gated {
        require(
            manifest,
            Permission::SecurityOpt,
            name,
            "security_opt",
            langs,
        )?;
        used.security = true;
    }
    if service.privileged == Some(true) {
        require(
            manifest,
            Permission::Privileged,
            name,
            "privileged: true",
            langs,
        )?;
        used.privileged = true;
    }
    Ok(())
}

fn validate_socket(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    used: &mut UsedPrivileges,
    langs: &Languages,
) -> Result<(), ServiceError> {
    for entry in &service.volumes {
        let Some((source, target, read_only)) = volume_mount(entry) else {
            continue;
        };
        if source == DOCKER_SOCKET || target == DOCKER_SOCKET {
            if source != DOCKER_SOCKET || target != DOCKER_SOCKET || read_only {
                return Err(ServiceError::new(
                    name,
                    text(langs, "plugin-install-priv-socket-mount", &[]),
                ));
            }
            require(
                manifest,
                Permission::DockerSocket,
                name,
                "docker.sock 挂载",
                langs,
            )?;
            used.socket = true;
        }
    }
    Ok(())
}

fn validate_used_permissions(
    manifest: &Manifest,
    used: &UsedPrivileges,
    langs: &Languages,
) -> Result<(), ServiceError> {
    check_declared(
        manifest,
        Permission::NetworkHost,
        used.host,
        "network_mode: host",
        langs,
    )?;
    check_declared(manifest, Permission::HostPid, used.pid, "pid: host", langs)?;
    check_declared(manifest, Permission::HostIpc, used.ipc, "ipc: host", langs)?;
    check_declared(
        manifest,
        Permission::SecurityOpt,
        used.security,
        "security_opt",
        langs,
    )?;
    check_declared(
        manifest,
        Permission::Privileged,
        used.privileged,
        "privileged: true",
        langs,
    )?;
    check_declared(
        manifest,
        Permission::DockerSocket,
        used.socket,
        "docker.sock 挂载",
        langs,
    )?;
    for permission in &manifest.permissions {
        match permission {
            Permission::Capability { name } if !used.caps.contains(name) => {
                return Err(ServiceError::new(
                    "*",
                    text(
                        langs,
                        "plugin-install-priv-missing-cap-add",
                        &[("permission", permission.to_string().into())],
                    ),
                ));
            }
            Permission::Device { path } if !used.devices.contains(path) => {
                return Err(ServiceError::new(
                    "*",
                    text(
                        langs,
                        "plugin-install-priv-missing-devices",
                        &[("permission", permission.to_string().into())],
                    ),
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
    langs: &Languages,
) -> Result<(), ServiceError> {
    if manifest.permissions.contains(&permission)
        || manifest.optional_permissions.contains_key(&permission)
    {
        Ok(())
    } else {
        Err(ServiceError::new(
            service,
            text(
                langs,
                "plugin-install-priv-field-requires",
                &[
                    ("field", field.into()),
                    ("permission", permission.to_string().into()),
                ],
            ),
        ))
    }
}

fn check_declared(
    manifest: &Manifest,
    permission: Permission,
    used: bool,
    field: &str,
    langs: &Languages,
) -> Result<(), ServiceError> {
    if (manifest.permissions.contains(&permission)
        || manifest.optional_permissions.contains_key(&permission))
        && !used
    {
        Err(ServiceError::new(
            "*",
            text(
                langs,
                "plugin-install-priv-missing-field",
                &[
                    ("permission", permission.to_string().into()),
                    ("field", field.into()),
                ],
            ),
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
pub fn apply(service: &Service, host: &mut HostConfig, langs: &Languages) -> Result<(), String> {
    if !service.cap_add.is_empty() {
        host.cap_add = Some(
            service
                .cap_add
                .iter()
                .map(|cap| {
                    normalize_capability(cap).ok_or_else(|| {
                        text(
                            langs,
                            "plugin-install-priv-cap-invalid",
                            &[("cap", cap.into())],
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    if !service.cap_drop.is_empty() {
        host.cap_drop = Some(
            service
                .cap_drop
                .iter()
                .map(|cap| {
                    normalize_capability(cap).ok_or_else(|| {
                        text(
                            langs,
                            "plugin-install-priv-cap-drop-invalid",
                            &[("cap", cap.into())],
                        )
                    })
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
        // 宿主网络里的容器带着 Docker 默认的 NET_RAW 能抓别人的包，默认去掉；插件已写 ALL 或 NET_RAW 就不重复。
        let drops = host.cap_drop.get_or_insert_with(Vec::new);
        if !drops.iter().any(|cap| cap == "NET_RAW" || cap == "ALL") {
            drops.push("NET_RAW".to_owned());
        }
    }
    let mut others = Vec::new();
    let mut no_new_privileges = None;
    for opt in &service.security_opt {
        match parse_security_opt(opt, langs)? {
            SecurityOpt::NoNewPrivileges(value) => no_new_privileges = Some(value),
            SecurityOpt::Other => others.push(opt.clone()),
        }
    }
    if !others.is_empty() {
        host.security_opt = Some(others);
    }
    // 插件容器默认 no-new-privileges:true；写了 no-new-privileges:false 才退回 Docker 默认。
    if no_new_privileges != Some(false) {
        host.security_opt
            .get_or_insert_with(Vec::new)
            .push("no-new-privileges:true".to_owned());
    }
    if service.pid.as_deref() == Some("host") {
        host.pid_mode = Some("host".to_owned());
    }
    if service.ipc.as_deref() == Some("host") {
        host.ipc_mode = Some("host".to_owned());
    }
    if service.privileged == Some(true) {
        host.privileged = Some(true);
    }
    for device in &service.devices {
        let (source, target, permissions) = device.paths(langs)?;
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
