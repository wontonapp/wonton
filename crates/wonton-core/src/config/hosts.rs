//! 核心对外暴露的主机名与按 Host 头分类的目标（ADR-0043 的域名约定）。

/// 插件经核心在插件网络里的这个别名访问 Matrix 客户端接口（`WONTON_MATRIX_HOMESERVER`）。`.internal` 是留给
/// 私有网络的顶级域名，基础域名不能落在它下面。
pub const PLUGIN_MATRIX_HOST: &str = "matrix.wonton.internal";

/// 插件经核心在插件网络里的这个别名访问身份提供方（`WONTON_OIDC_ISSUER` 和后台端点，ADR-0063）。有的库把
/// `*.localhost` 直接当回环，`auth.<base>` 的别名对它们不起作用。
pub const PLUGIN_AUTH_HOST: &str = "auth.wonton.internal";

/// 插件经核心在插件网络里的这个别名访问模型网关（`WONTON_MODEL_BASE_URL` 等）。
pub const PLUGIN_MODELS_HOST: &str = "models.wonton.internal";

/// 插件经核心在插件网络里的这个别名发通知（`WONTON_NOTIFY_URL`，ADR-0047）。
pub const PLUGIN_NOTIFY_HOST: &str = "notify.wonton.internal";

/// 申请了 `notify` 的插件经这个 Mailpit 别名向家里人发信（ADR-0056）。
pub const PLUGIN_SMTP_HOST: &str = "smtp.wonton.internal";

/// Tuwunel 经核心私有网络上的这个别名直连推送专用监听（ADR-0003 的“接口：推送”）。
pub const PUSH_GATEWAY_HOST: &str = "push.wonton.internal";

/// 插件经核心在插件网络里的这个别名访问 MCP 总线（`WONTON_MCP_URL`）。
pub const PLUGIN_MCP_HOST: &str = "mcp.wonton.internal";

/// 申请了 `plugins.health` 的插件经核心在插件网络里的这个别名读所有插件的健康状态（规范“插件健康探测”）。
pub const PLUGIN_HEALTH_HOST: &str = "health.wonton.internal";

/// tusd 经核心在私有网络里的这个别名调文件钩子（ADR-0020）。
pub const FILE_HOOKS_HOST: &str = "files-hooks.wonton.internal";
/// 网关按 Host 头区分的目标。
#[derive(Debug, Clone, PartialEq, Eq)]

pub enum HostKind {
    Core,

    /// 浏览器走 `auth.<base>`、插件走 [`PLUGIN_AUTH_HOST`]：都是身份提供方的公开端点。
    Auth,

    Matrix,

    Plugin(String),

    /// 插件网络里的 Bot 访问 Matrix（[`PLUGIN_MATRIX_HOST`]）。
    PluginMatrix,

    /// 插件网络里的插件访问模型网关（[`PLUGIN_MODELS_HOST`]）。
    PluginModels,

    /// 插件网络里的插件发通知（[`PLUGIN_NOTIFY_HOST`]，ADR-0047）。
    PluginNotify,

    /// Tuwunel 调推送代理（[`PUSH_GATEWAY_HOST`]）。
    Push,

    /// 局域网浏览器用服务器的 IP 地址访问：状态页。
    Lan(std::net::IpAddr),

    /// 插件网络里的 Agent 访问 MCP 总线（[`PLUGIN_MCP_HOST`]）。
    PluginMcp,

    /// 插件网络里的插件读健康状态（[`PLUGIN_HEALTH_HOST`]）。
    PluginHealth,

    /// tusd 调文件钩子（[`FILE_HOOKS_HOST`]）。
    FileHooks,

    Unknown,
}

/// 浏览器看到的地址：核心在 `<base>`，身份提供方在 `auth.<base>`，Matrix 客户端接口在 `matrix.<base>`，
/// 插件在 `<slug>.<base>`。插件的 slug 至少两段（含 `-`），不会和 `auth`、`matrix` 重名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrls {
    pub scheme: String,
    pub base_domain: String,
    pub port: u16,
}

impl PublicUrls {
    pub fn core_host(&self) -> &str {
        &self.base_domain
    }

    pub fn auth_host(&self) -> String {
        format!("auth.{}", self.base_domain)
    }

    /// 插件网络里身份提供方的地址（[`PLUGIN_AUTH_HOST`]），端口同网关。
    pub fn auth_internal_origin(&self) -> String {
        self.origin(PLUGIN_AUTH_HOST)
    }

    pub fn matrix_host(&self) -> String {
        format!("matrix.{}", self.base_domain)
    }

    pub fn plugin_host(&self, slug: &str) -> String {
        format!("{slug}.{}", self.base_domain)
    }

    /// `scheme://host[:port]`，默认端口省略。
    pub fn origin(&self, host: &str) -> String {
        let default_port = matches!(
            (self.scheme.as_str(), self.port),
            ("http", 80) | ("https", 443)
        );

        if default_port {
            format!("{}://{host}", self.scheme)
        } else {
            format!("{}://{host}:{}", self.scheme, self.port)
        }
    }

    pub fn classify(&self, host_header: &str) -> HostKind {
        let host = host_header
            .rsplit_once(':')
            .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
            .map_or(host_header, |(host, _)| host)
            .trim_end_matches('.')
            .to_ascii_lowercase();

        if host == self.base_domain {
            return HostKind::Core;
        }

        if let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse() {
            return HostKind::Lan(ip);
        }

        if host == PLUGIN_AUTH_HOST {
            return HostKind::Auth;
        }

        if host == PLUGIN_MATRIX_HOST {
            return HostKind::PluginMatrix;
        }

        if host == PLUGIN_MODELS_HOST {
            return HostKind::PluginModels;
        }

        if host == PLUGIN_NOTIFY_HOST {
            return HostKind::PluginNotify;
        }

        if host == PUSH_GATEWAY_HOST {
            return HostKind::Push;
        }

        if host == PLUGIN_MCP_HOST {
            return HostKind::PluginMcp;
        }

        if host == PLUGIN_HEALTH_HOST {
            return HostKind::PluginHealth;
        }

        if host == FILE_HOOKS_HOST {
            return HostKind::FileHooks;
        }

        match host.strip_suffix(&format!(".{}", self.base_domain)) {
            Some("auth") => HostKind::Auth,

            Some("matrix") => HostKind::Matrix,

            Some(slug) if !slug.is_empty() && !slug.contains('.') => {
                HostKind::Plugin(slug.to_owned())
            }

            _ => HostKind::Unknown,
        }
    }
}
