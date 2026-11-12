//! 渲染入口 HAProxy 配置要的输入：名字、标签和真实来源怎么来，都是核心定的值。

use super::labels;

pub const PUBLIC_BIND: &str = "entry-edge.wonton.internal:8443";
pub const LAN_BIND: &str = "entry-lanfwd.wonton.internal:8443";
pub const PAIRED_BIND: &str = "entry.wonton.internal:8449";
/// 运行时接口（`show table`、`clear table`）：只在 `wonton-clients` 上的地址可达。
pub const RUNTIME_SOCKET: &str = "entry.wonton.internal:9999";
pub const GATEWAY_ADDRESS: &str = "entry-gateway.wonton.internal:8484";

/// `wonton-edge` 的子网：连接器要固定地址，`tcp-request connection` 的条件在连接层求值，
/// 那里只认 IP、不解析名字。入口自己也固定在这个网段里。
pub const EDGE_SUBNET: &str = "10.90.0.0/24";
/// 连接器在 `wonton-edge` 上的固定地址。连接器只有 PROXY v2 和 Cloudflare 两种来源写法，
/// 两种都只认这个对端。
pub const CONNECTOR_ADDRESS: &str = "10.90.0.2";
pub const ENTRY_EDGE_ADDRESS: &str = "10.90.0.3";
/// 局域网转发在 `wonton-lanfwd` 上的固定地址（C8 建网）。连接器不在那个网络上（ADR-0021“网络规则”）。
pub const LAN_FORWARD_ADDRESS: &str = "10.90.1.2";
pub const LAN_FORWARD_SUBNET: &str = "10.90.1.0/24";

/// 证书文件（证书和私钥在同一个 PEM 里），和 crt-list 一样在只读的 `/certs` 下。
pub const CERT_FILE: &str = "/certs/site.pem";
pub const PUBLIC_CRT_LIST: &str = "/certs/public.crt-list";
pub const LAN_CRT_LIST: &str = "/certs/lan.crt-list";

/// 一个来源 5 分钟内失败超过这个数就回 429（ADR-0021“防暴力破解”）。
pub const AUTH_FAILURES_LIMIT: u32 = 10;
/// 表项从最后一次失败起留这么久，也就是封禁时长。
pub const AUTH_FAILURES_EXPIRE: &str = "15m";
pub const AUTH_FAILURES_SIZE: &str = "100k";

/// 核心给响应加、入口用来计数的内部头，回给客户端前删掉。
pub const AUTH_FAILURE_HEADER: &str = "X-Wonton-Auth-Failure";

/// 公网前端怎么拿到真实来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// PROXY v2：frpc 和 TCP 转发连接器。只认连接器，其余连接不解析 PROXY 头。
    ProxyV2,
    /// 边缘（Cloudflare Tunnel）写在请求头里，`Header` 是头名。不读 PROXY 头。
    Header(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// 证书的父域，如 `k3f9qzab.example.test`。标签硬拼在它前面。
    pub suffix: String,
    /// 联邦的服务器名，进公网前端的 crt-list（公网入口关着时它也照常要能握手）。
    pub federation: Option<String>,
    /// 生成 8449 的配对前端（已配对的云吞之间）。
    pub paired_federation: bool,
    /// 入口自检用的标签，也不算进任何插件的配额。
    pub probe_label: String,
    /// 主人打开了公网入口的插件标签。
    pub public_labels: Vec<String>,
    /// 主人打开了局域网入口的插件标签。
    pub lan_labels: Vec<String>,
    pub trust: Trust,
    /// 公网前端认的对端（连接器）。局域网转发是 `lan_forward`，不在这个列表里。
    pub connectors: Vec<String>,
    /// 入口容器接了局域网转发网络时才有；没有就不生成局域网前端（C8 之前）。
    pub lan_forward: Option<String>,
}

impl Model {
    /// 只有联邦、没有任何插件标签的形状：`entry_compose` 在 C5 之前生成的配置。
    pub fn federation_only(suffix: &str, federation: &str) -> Self {
        Self {
            suffix: suffix.to_owned(),
            federation: Some(federation.to_owned()),
            paired_federation: true,
            probe_label: labels::PROBE_LABEL.to_owned(),
            public_labels: Vec::new(),
            lan_labels: Vec::new(),
            trust: Trust::ProxyV2,
            connectors: vec![CONNECTOR_ADDRESS.to_owned()],
            lan_forward: None,
        }
    }

    /// 公网前端 crt-list 里的名字：联邦、探测标签、打开的公网入口。
    pub fn public_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(federation) = &self.federation {
            names.push(federation.clone());
        }
        names.push(self.with_suffix(&self.probe_label));
        names.extend(self.public_labels.iter().map(|l| self.with_suffix(l)));
        names
    }

    /// 局域网前端 crt-list 里的名字。
    pub fn lan_names(&self) -> Vec<String> {
        self.lan_labels
            .iter()
            .map(|label| format!("{label}.lan.{}", self.suffix))
            .collect()
    }

    fn with_suffix(&self, label: &str) -> String {
        format!("{label}.{}", self.suffix)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_suffix(&self.suffix)?;
        if let Some(federation) = &self.federation {
            validate_suffix(federation)?;
        }
        labels::validate_name(&self.probe_label)?;
        for label in &self.public_labels {
            labels::validate_label(label)?;
            if *label == self.probe_label {
                return Err(format!("公网标签 {label} 和探测标签重名"));
            }
        }
        for label in &self.lan_labels {
            labels::validate_label(label)?;
            if *label == self.probe_label {
                return Err(format!("局域网标签 {label} 和探测标签重名"));
            }
        }
        let mut public = self.public_labels.clone();
        public.sort();
        public.dedup();
        if public.len() != self.public_labels.len() {
            return Err("公网标签有重复".to_owned());
        }
        let mut lan = self.lan_labels.clone();
        lan.sort();
        lan.dedup();
        if lan.len() != self.lan_labels.len() {
            return Err("局域网标签有重复".to_owned());
        }
        if let Trust::Header(name) = &self.trust {
            validate_header_name(name)?;
        }
        if self.connectors.is_empty() {
            return Err("公网前端至少要一个连接器地址".to_owned());
        }
        for connector in &self.connectors {
            validate_peer(connector)?;
        }
        if let Some(forward) = &self.lan_forward {
            validate_peer(forward)?;
        }
        Ok(())
    }
}

/// 证书名：小写的点分域名，长度和每段的字符集和标签一样。
pub fn validate_suffix(suffix: &str) -> Result<(), String> {
    if suffix.is_empty() || suffix.len() > 253 {
        return Err(format!("后缀 {suffix:?} 长度不在 1..=253 字节"));
    }
    let segments: Vec<&str> = suffix.split('.').collect();
    for segment in segments {
        if !is_label(segment) {
            return Err(format!("后缀 {suffix:?} 的段 {segment:?} 不合法"));
        }
    }
    Ok(())
}

/// 请求头名：RFC 9110 的 token 里去掉 `%`、括号和逗号（这些会进 `hdr()` 表达式）。
pub fn validate_header_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!("请求头名 {name:?} 不合法"))
    }
}

/// `{ src … }` 里的对端：连接层只认 IP，不写端口。
pub fn validate_peer(peer: &str) -> Result<(), String> {
    if peer.parse::<std::net::IpAddr>().is_ok() {
        Ok(())
    } else {
        Err(format!("对端地址 {peer:?} 不是 IP"))
    }
}

fn is_label(segment: &str) -> bool {
    (1..=63).contains(&segment.len())
        && segment.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
