//! 不跑代码的两种形态的清单字段（ADR-0080）：`static_web` 的静态文件目录和出网白名单、`link` 的地址。

use serde::{Deserialize, Serialize};

use super::manifest::{Manifest, ManifestError, Permission, Runtime};

/// 静态网页出网白名单最多几项。
pub const MAX_CONNECT: usize = 32;

/// `x-wonton.static_web`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticWebSpec {
    /// 包里放静态文件的目录。
    #[serde(default = "default_root")]
    pub root: String,
    /// 找不到的路径回这个文件（单页应用）；没写时回 404。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// 页面能连的外部来源：`https://<主机名>` 或 `wss://<主机名>`，主机名可以以 `*.` 开头。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect: Vec<String>,
}

impl Default for StaticWebSpec {
    fn default() -> Self {
        Self {
            root: default_root(),
            fallback: None,
            connect: Vec::new(),
        }
    }
}

fn default_root() -> String {
    "web".to_owned()
}

/// `x-wonton.link`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkSpec {
    pub url: String,
}

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

/// 形态和字段对得上，以及两种形态各自不能写的字段。`services` 由 [`super::package`] 按形态检查。
pub(super) fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.static_web.is_some() && manifest.runtime != Runtime::StaticWeb {
        return Err(invalid(
            "static_web 只能和 runtime: static_web 一起写".to_owned(),
        ));
    }
    if manifest.link.is_some() != (manifest.runtime == Runtime::Link) {
        return Err(invalid(
            "runtime: link 必须写 link.url，link 也只能和 runtime: link 一起写".to_owned(),
        ));
    }
    match manifest.runtime {
        Runtime::StaticWeb => validate_static_web(manifest),
        Runtime::Link => validate_link(manifest),
        _ => Ok(()),
    }
}

/// 没有服务端的形态写了这些字段就不合法：不静默忽略（ADR-0080 第 3、5 节）。
fn server_only_fields(manifest: &Manifest) -> Vec<&'static str> {
    [
        ("secrets", !manifest.secrets.is_empty()),
        ("settings", manifest.settings.is_some()),
        ("mcp", manifest.mcp.is_some()),
        ("bot", manifest.bot.is_some()),
        ("bots", manifest.bots.is_some()),
        ("agent", manifest.agent.is_some()),
        ("provides", manifest.provides.is_some()),
        ("lan", manifest.lan.is_some()),
        ("tunnel", manifest.tunnel.is_some()),
        ("setup", manifest.setup.is_some()),
        ("gpu", manifest.gpu.is_some()),
        (
            "optional_permissions",
            !manifest.optional_permissions.is_empty(),
        ),
        // 静态网页的展示字段（不带 service、port 的 webui）随托管实现放开。
        ("webui", manifest.webui.is_some()),
        ("tabs", manifest.tabs.is_some()),
    ]
    .into_iter()
    .filter_map(|(field, declared)| declared.then_some(field))
    .collect()
}

fn reject_fields(runtime: &str, fields: &[&str]) -> Result<(), ManifestError> {
    if fields.is_empty() {
        return Ok(());
    }
    Err(invalid(format!(
        "runtime: {runtime} 没有服务端，用不上 {}；要用就写成容器插件",
        fields.join("、")
    )))
}

fn validate_static_web(manifest: &Manifest) -> Result<(), ManifestError> {
    reject_fields("static_web", &server_only_fields(manifest))?;
    let spec = manifest.static_web.clone().unwrap_or_default();
    if !is_package_path(&spec.root) {
        return Err(invalid(format!(
            "static_web.root `{}` 要是包里的相对路径，不能有 ..、以 . 开头的段或 _wonton",
            spec.root
        )));
    }
    if let Some(fallback) = &spec.fallback
        && !is_package_path(fallback)
    {
        return Err(invalid(format!(
            "static_web.fallback `{fallback}` 要是 static_web.root 下的相对路径"
        )));
    }
    if spec.connect.len() > MAX_CONNECT {
        return Err(invalid(format!("static_web.connect 最多 {MAX_CONNECT} 项")));
    }
    if let Some(bad) = spec
        .connect
        .iter()
        .find(|origin| !is_connect_origin(origin))
    {
        return Err(invalid(format!(
            "static_web.connect `{bad}` 只能写 https://<主机名> 或 wss://<主机名>（可以 *. 开头），\
             不带路径、端口，不能是 IP 地址或 localhost"
        )));
    }
    if let Some(name) = manifest
        .requires
        .iter()
        .chain(&manifest.wants)
        .find(|name| *name != super::capabilities::WEB_STORAGE.name)
    {
        return Err(invalid(format!(
            "runtime: static_web 只能依赖 web_storage，不能依赖 {name}"
        )));
    }
    let egress = manifest.permissions.contains(&Permission::NetworkEgress);
    if let Some(other) = manifest
        .permissions
        .iter()
        .find(|permission| **permission != Permission::NetworkEgress)
    {
        return Err(invalid(format!(
            "runtime: static_web 只能申请 network.egress，不能申请 {other}"
        )));
    }
    if egress == spec.connect.is_empty() {
        return Err(invalid(
            "static_web.connect 和 network.egress 权限要一起写".to_owned(),
        ));
    }
    Ok(())
}

fn validate_link(manifest: &Manifest) -> Result<(), ManifestError> {
    let mut fields = server_only_fields(manifest);
    if !manifest.permissions.is_empty() {
        fields.push("permissions");
    }
    if !manifest.requires.is_empty() || !manifest.wants.is_empty() {
        fields.push("requires/wants");
    }
    reject_fields("link", &fields)?;
    let url = manifest.link.as_ref().map(|link| link.url.as_str());
    let parsed = url.and_then(|url| url::Url::parse(url).ok());
    let ok = parsed.as_ref().is_some_and(|parsed| {
        parsed.scheme() == "https"
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
    });
    if !ok {
        return Err(invalid(format!(
            "link.url `{}` 要是 https 地址，不能带用户名和密码",
            url.unwrap_or_default()
        )));
    }
    Ok(())
}

/// 包里的相对路径：非空，不以 `/` 开头，没有 `..`、`.` 开头的段和 `_wonton` 段（网关占用）。
fn is_package_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && part != "_wonton")
}

/// `https://<主机名>` 或 `wss://<主机名>`：主机名至少两段，可以以 `*.` 开头，不是 IP 字面量。
fn is_connect_origin(origin: &str) -> bool {
    let Some(host) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("wss://"))
    else {
        return false;
    };
    let host = host.strip_prefix("*.").unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| is_dns_label(label))
        // 最后一段全是数字的是 IPv4 字面量，不是主机名。
        && !labels.last().is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()))
}

fn is_dns_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
