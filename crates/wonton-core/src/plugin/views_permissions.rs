//! 插件权限给 App 的文案：说明、风险级别和高权限的提示。安装确认页、Bot 的确认卡片和插件目录的
//! 搜索结果都用它。

use wonton_protocol::{PermissionLine, PermissionRisk};

use super::languages::Languages;
use super::manager::supported;
use super::manifest::{Access, LocalizedText, Manifest, Permission};

/// 权限的说明：安装确认页、Bot 的确认卡片和插件目录的搜索结果都用它。
pub fn permission_text(permission: &Permission) -> String {
    let access = |access: &Access| match access {
        Access::Read => "读",
        Access::Write => "读写",
    };
    match permission {
        Permission::StorageShare { name, access: a } => format!("{}共享文件夹 {name}", access(a)),
        Permission::StorageHome { access: a } => format!("{}你的私人空间", access(a)),
        Permission::Models => "使用模型网关".to_owned(),
        Permission::Tools => "调用核心和其他应用的工具".to_owned(),
        Permission::MatrixBot => "以 Bot 身份收发消息".to_owned(),
        Permission::Notify => "给你发通知".to_owned(),
        Permission::OwnerIdentity => "知道你的用户名和邮箱".to_owned(),
        Permission::NetworkEgress => "访问外网和局域网".to_owned(),
        Permission::EgressProxy => "提供出网代理（高权限）".to_owned(),
        Permission::NetworkLan => "在局域网上开放端口（高权限）".to_owned(),
        Permission::NetworkHost => "使用宿主的网络（很高权限）".to_owned(),
        Permission::Capability { name } => format!("增加内核能力 {name}"),
        Permission::HostPid => "共享宿主进程空间（最高权限）".to_owned(),
        Permission::HostIpc => "共享宿主 IPC（最高权限）".to_owned(),
        Permission::SecurityOpt => "修改容器安全选项（最高权限）".to_owned(),
        Permission::Privileged => "以特权模式运行（最高权限）".to_owned(),
        Permission::DevicesGpu => "使用 GPU（高权限）".to_owned(),
        Permission::Device { path } => format!("访问宿主设备 {path}（高权限）"),
        Permission::DockerSocket => "控制 Docker（最高权限）".to_owned(),
    }
}

fn permission_text_en(permission: &Permission) -> String {
    let access = |access: &Access| match access {
        Access::Read => "Read",
        Access::Write => "Read and write",
    };
    match permission {
        Permission::StorageShare { name, access: a } => {
            format!("{} the shared folder {name}", access(a))
        }
        Permission::StorageHome { access: a } => format!("{} your private space", access(a)),
        Permission::Models => "Use the model gateway".to_owned(),
        Permission::Tools => "Call tools of the core and other plugins".to_owned(),
        Permission::MatrixBot => "Send and receive messages as a bot".to_owned(),
        Permission::Notify => "Send you notifications".to_owned(),
        Permission::OwnerIdentity => "Know your username and email".to_owned(),
        Permission::NetworkEgress => "Reach the internet and your local network".to_owned(),
        Permission::EgressProxy => "Provide an egress proxy (high permission)".to_owned(),
        Permission::NetworkLan => "Open ports on your local network (high permission)".to_owned(),
        Permission::NetworkHost => "Use the host's network (very high permission)".to_owned(),
        Permission::Capability { name } => format!("Add the kernel capability {name}"),
        Permission::HostPid => "Share the host process namespace (highest permission)".to_owned(),
        Permission::HostIpc => "Share the host IPC namespace (highest permission)".to_owned(),
        Permission::SecurityOpt => {
            "Change container security options (highest permission)".to_owned()
        }
        Permission::Privileged => "Run in privileged mode (highest permission)".to_owned(),
        Permission::DevicesGpu => "Use the GPU (high permission)".to_owned(),
        Permission::Device { path } => format!("Access the host device {path} (high permission)"),
        Permission::DockerSocket => "Control Docker (highest permission)".to_owned(),
    }
}

fn permission_risk(permission: &Permission) -> Option<PermissionRisk> {
    match permission {
        Permission::EgressProxy
        | Permission::NetworkLan
        | Permission::DevicesGpu
        | Permission::Device { .. } => Some(PermissionRisk::High),
        Permission::NetworkHost => Some(PermissionRisk::VeryHigh),
        Permission::Capability { name } => Some(match super::privileges::capability_risk(name) {
            super::privileges::CapabilityRisk::High => PermissionRisk::High,
            super::privileges::CapabilityRisk::VeryHigh => PermissionRisk::VeryHigh,
            super::privileges::CapabilityRisk::Highest => PermissionRisk::Highest,
        }),
        Permission::HostPid
        | Permission::HostIpc
        | Permission::SecurityOpt
        | Permission::Privileged
        | Permission::DockerSocket => Some(PermissionRisk::Highest),
        _ => None,
    }
}

fn permission_notice(permission: &Permission, langs: &Languages) -> Option<String> {
    let text = match permission {
        Permission::EgressProxy => LocalizedText::from([
            ("zh".to_owned(), "选它做出网代理后，所有出网插件的请求都经过它：它能看到这些插件访问了哪些网站，也能改写不加密的内容。".to_owned()),
            ("en".to_owned(), "When you select it as your egress proxy, all requests from plugins with network access pass through it: it can see which websites they visit and modify unencrypted content.".to_owned()),
        ]),
        Permission::NetworkHost => LocalizedText::from([
            ("zh".to_owned(), "这个插件能直接访问你家里的网络和本机所有端口，包括只给本机用的服务，也能绕过 Wonton 的登录连到部分其他插件。它开的端口会出现在本机所有网卡上，包括 Wi-Fi、Tailscale 和 IPv6 地址，本机有公网地址时互联网也能连上。本机开着防火墙时，局域网设备连不进来，要你自己放行。".to_owned()),
            ("en".to_owned(), "This plugin can directly access your home network and all ports on this machine, including services bound only to localhost, and bypass Wonton login to connect to some other plugins. Its ports will appear on all network interfaces, including Wi-Fi, Tailscale, and IPv6 addresses; if this machine has a public address, the internet can reach them too. If the machine firewall is on, LAN devices cannot connect until you allow them yourself.".to_owned()),
        ]),
        Permission::HostPid
        | Permission::HostIpc
        | Permission::SecurityOpt
        | Permission::Privileged
        | Permission::DockerSocket
        | Permission::Capability { .. }
            if permission_risk(permission) == Some(PermissionRisk::Highest) =>
        {
            LocalizedText::from([
                ("zh".to_owned(), "这个插件能控制整台主机：读写所有文件和其他插件的数据，改动或停掉 Wonton 自己。这等于把主机的 root 交给它。只装你信任的插件。卸载后，它建的容器和改过的东西不会被清理。".to_owned()),
                ("en".to_owned(), "This plugin can control the entire host: read and write all files and other plugins' data, change or stop Wonton itself. This is equivalent to giving it root on the host. Only install plugins you trust. After uninstalling, containers it created and things it changed will not be cleaned up.".to_owned()),
            ])
        }
        Permission::NetworkLan => LocalizedText::from([
            ("zh".to_owned(), "局域网里的设备可以直接连接插件开放的端口，不经 Wonton 登录；身份验证由插件自己负责。".to_owned()),
            ("en".to_owned(), "Devices on your local network can connect directly to the plugin's ports without Wonton login; the plugin is responsible for authentication.".to_owned()),
        ]),
        Permission::Device { path } => LocalizedText::from([
            ("zh".to_owned(), format!("这个插件可以访问宿主设备 {path}。")),
            ("en".to_owned(), format!("This plugin can access the host device {path}.")),
        ]),
        _ => return None,
    };
    Some(langs.pick_owned(&text))
}

pub fn permission_line(permission: &Permission, langs: &Languages) -> PermissionLine {
    let text = LocalizedText::from([
        ("zh".to_owned(), permission_text(permission)),
        ("en".to_owned(), permission_text_en(permission)),
    ]);
    PermissionLine {
        permission: permission.to_string(),
        text: langs.pick_owned(&text),
        risk: permission_risk(permission),
        notice: permission_notice(permission, langs),
        supported: supported(permission),
        added: false,
    }
}

/// 应用拿不拿得到主人的身份：容器里的变量之外，还有登录时给的 OIDC 声明和代登录模板（ADR-0022）。
pub(super) fn reveals_owner_identity(m: &Manifest) -> bool {
    m.gets_owner_identity()
        || m.webui
            .as_ref()
            .is_some_and(|w| w.sso == super::manifest::Sso::Oidc)
        || super::proxy_auth::uses_identity(m)
}
