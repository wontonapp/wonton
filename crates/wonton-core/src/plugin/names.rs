//! 插件的 Docker 资源名和标签（ADR-0011 的“资源名”）。几套核心共用一台 Docker 时（插件开发环境，ADR-0017），
//! 每套核心设一个实例名（`WONTON_INSTANCE`），资源名和标签都带上它，各管各的插件；正式安装没有实例名。

use std::collections::HashMap;

use super::manifest::PluginId;

/// 资源属于哪个插件。
pub const LABEL_PLUGIN: &str = "wonton.plugin";
/// 资源属于哪套核心：只有设了实例名的核心才打这个标签。
pub const LABEL_INSTANCE: &str = "wonton.instance";
/// 核心管理的其他组件（ADR-0025）：切到不支持这个组件的核心镜像之前，升级器按标签删掉容器。这两个标签的名字和含义是跨版本的接口。
pub const LABEL_MANAGED_BY: &str = "wonton.managed-by";
pub const LABEL_COMPONENT: &str = "wonton.core.component";
/// 出口容器（ADR-0023）这个组件的名字。
pub const EGRESS_COMPONENT: &str = "wonton-egress";

const MAX_INSTANCE_LEN: usize = 16;

/// 插件服务域名共享的后缀。基础域名不能用它（`PublicUrls` 校验），核心在插件网络里的别名（`mcp.wonton.internal`
/// 等）也不会撞上：slug 至少两段、都含连字符。
pub const SERVICE_DOMAIN_SUFFIX: &str = "wonton.internal";

/// 安装时注入 `WONTON_SERVICE_DOMAIN` 的值：`<slug>.wonton.internal`。作者在 compose 里写
/// `mcp.${WONTON_SERVICE_DOMAIN}` 放进上游服务的 Host 白名单。
pub fn service_domain(slug: &str) -> String {
    format!("{slug}.{SERVICE_DOMAIN_SUFFIX}")
}

/// 插件服务在它所在插件网络里的稳定地址名：`<服务名>.<slug>.wonton.internal`。核心连插件的网页、MCP 和
/// internal_url 用它当 Host，这个别名加在每个插件服务的容器上。和实例名无关：网络本来就按实例分开。
pub fn service_host(slug: &str, service: &str) -> String {
    format!("{service}.{}", service_domain(slug))
}

/// 核心的实例名，没有时是正式安装。实例名只有小写字母和数字：插件的 slug 至少两段、都含连字符，所以
/// `wonton-<实例名>--…` 和正式安装的 `wonton-<slug>…` 不会重名，不同实例之间也不会。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instance(Option<String>);

impl Instance {
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        let Some(name) = name else {
            return Ok(Self(None));
        };
        let valid = !name.is_empty()
            && name.len() <= MAX_INSTANCE_LEN
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if !valid {
            return Err(format!(
                "实例名 `{name}` 不合法：只能用小写字母和数字，最长 {MAX_INSTANCE_LEN} 个字符"
            ));
        }
        Ok(Self(Some(name.to_owned())))
    }

    /// 实例名；正式安装时没有。
    pub fn name(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// 插件网络 `wonton-<slug>`，有实例名时 `wonton-<实例名>--<slug>`；也是插件容器和命名卷的名字前缀。
    pub fn network(&self, slug: &str) -> String {
        match &self.0 {
            None => format!("wonton-{slug}"),
            Some(instance) => format!("wonton-{instance}--{slug}"),
        }
    }

    /// 容器名 `<插件网络>--<服务名>`。slug 由单个 `-` 连接的小写字母数字段组成，不含 `--`，也不以 `-`
    /// 结尾，所以 slug 后的第一个 `--` 就是分隔符，不同插件的容器不会重名。只用一个 `-` 时会重名：`app.foo`
    /// 的服务 `bar-web` 和 `app.foo.bar` 的服务 `web` 都是 `wonton-app-foo-bar-web`。
    pub fn container(&self, slug: &str, service: &str) -> String {
        format!("{}--{service}", self.network(slug))
    }

    /// 命名卷 `<插件网络>--<卷名>`，理由同容器名。
    pub fn volume(&self, slug: &str, volume: &str) -> String {
        format!("{}--{volume}", self.network(slug))
    }

    /// 没声明 `network.egress` 的插件发布局域网端口的网络 `<插件网络>--lan`（ADR-0019）。网络和容器不同名字空间，
    /// 和服务 `lan` 的容器同名也不冲突。
    pub fn lan_network(&self, slug: &str) -> String {
        format!("{}--lan", self.network(slug))
    }

    /// 检测局域网地址的一次性容器。名字里没有 `--`，不会和插件的容器重名。
    /// 问 NVIDIA 驱动的一次性容器（见 [`super::host_facts::ask_nvidia`]）。
    pub fn gpu_probe(&self) -> String {
        match &self.0 {
            None => "wonton-gpu-probe".to_owned(),
            Some(instance) => format!("wonton-{instance}-gpu-probe"),
        }
    }

    pub fn lan_probe(&self) -> String {
        match &self.0 {
            None => "wonton-lan-probe".to_owned(),
            Some(instance) => format!("wonton-{instance}-lan-probe"),
        }
    }

    /// 核心自己建的、不属于哪个插件的资源（如检测局域网地址的容器）的标签。
    pub fn core_labels(&self) -> HashMap<String, String> {
        let instance = self
            .0
            .iter()
            .map(|i| (LABEL_INSTANCE.to_owned(), i.clone()));
        instance.collect()
    }

    /// 出口容器和它的网络 `wonton-egress`，有实例名时 `wonton-<实例名>--egress`。slug 至少两段，不会和插件重名。
    pub fn egress(&self) -> String {
        self.network("egress")
    }

    /// 查代理为什么连不上的一次性容器。名字里没有 `--`，不会和插件的容器重名。
    pub fn egress_check(&self) -> String {
        match &self.0 {
            None => "wonton-egress-check".to_owned(),
            Some(instance) => format!("wonton-{instance}-egress-check"),
        }
    }

    /// 出口容器和它的网络的标签。
    pub fn component_labels(&self) -> HashMap<String, String> {
        let mut labels = self.core_labels();
        labels.insert(LABEL_MANAGED_BY.to_owned(), "core".to_owned());
        labels.insert(LABEL_COMPONENT.to_owned(), EGRESS_COMPONENT.to_owned());
        labels
    }

    /// 带这些标签的资源是不是这套核心的出口容器或它的网络。
    pub fn is_component(&self, labels: &HashMap<String, String>) -> bool {
        self.owns(labels)
            && labels.get(LABEL_MANAGED_BY).map(String::as_str) == Some("core")
            && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(EGRESS_COMPONENT)
    }

    /// 核心给插件的容器、网络和命名卷打的标签。
    pub fn labels(&self, id: &PluginId) -> HashMap<String, String> {
        let mut labels = HashMap::from([(LABEL_PLUGIN.to_owned(), id.to_string())]);
        if let Some(instance) = &self.0 {
            labels.insert(LABEL_INSTANCE.to_owned(), instance.clone());
        }
        labels
    }

    /// 带这些标签的资源是不是这套核心的：实例标签和实例名一致，正式安装的核心只认没有实例标签的。
    pub fn owns(&self, labels: &HashMap<String, String>) -> bool {
        labels.get(LABEL_INSTANCE).map(String::as_str) == self.0.as_deref()
    }
}

/// 正式安装时插件的容器名。
#[cfg(test)]
pub fn container_name(slug: &str, service: &str) -> String {
    Instance::default().container(slug, service)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names_keep_resources_apart() {
        let formal = Instance::default();
        let dev = Instance::parse(Some("alice2")).unwrap();
        assert_eq!(formal.container("app-foo", "web"), "wonton-app-foo--web");
        assert_eq!(dev.network("app-foo"), "wonton-alice2--app-foo");
        assert_eq!(dev.volume("app-foo", "db"), "wonton-alice2--app-foo--db");

        let id = PluginId::parse("app.foo").unwrap();
        assert!(formal.owns(&formal.labels(&id)));
        assert!(dev.owns(&dev.labels(&id)));
        assert!(!formal.owns(&dev.labels(&id)));
        assert!(!dev.owns(&formal.labels(&id)));
        let other = Instance::parse(Some("bob")).unwrap();
        assert!(!other.owns(&dev.labels(&id)));

        assert_eq!(Instance::parse(None).unwrap(), formal);
        let too_long = "a".repeat(MAX_INSTANCE_LEN + 1);
        for bad in ["", "dev-1", "Dev", "dev_1", "dev.1", too_long.as_str()] {
            assert!(Instance::parse(Some(bad)).is_err(), "{bad}");
        }
    }
}
