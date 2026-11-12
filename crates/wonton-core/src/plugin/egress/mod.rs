//! 出网插件走宿主的代理（ADR-0023）：主人配了代理时，声明了 `network.egress` 的插件经出口容器出去。
//!
//! - 这里：注入给插件的环境变量和插值变量；
//! - [`healthcheck`]：改写健康检查，让它不走代理；
//! - [`settings`]、[`address`]：用哪个代理；
//! - [`gost`]、[`lists`]、[`relay`]：出口容器，它的配置和名单，核心怎么管它；
//! - [`switch`]：第一次配上或取消代理时重建出网插件；
//! - [`check`]、[`probe`]：探测，连不上时查原因；`api`：管理接口。

pub mod address;
mod api;
pub mod check;
pub mod core_proxy;
pub mod gost;
pub mod healthcheck;
pub mod lists;
pub mod probe;
pub(in crate::plugin) mod provider;
mod relay;
pub mod settings;
mod switch;
pub(in crate::plugin) use switch::{PLACEHOLDER, cannot_read, token_uses};

use std::collections::BTreeSet;

use bollard::models::ContainerCreateBody;

/// 给有自己代理设置、不读 `HTTP_PROXY` 的程序用的插值变量。声明了 `network.egress` 时注入，没配代理时是空串。
pub const URL_VAR: &str = "WONTON_PROXY_URL";

/// 出口容器监听代理的端口。
pub const PROXY_PORT: u16 = 3128;

/// 插件经出口容器的这个地址出去。它不含宿主的地址，换代理不用改插件。
pub const PROXY_URL: &str = "http://egress.wonton.internal:3128";

/// 代理变量的名字。大小写都写：apt、wget 和 curl 的明文 HTTP 只认小写，只写大写时这些程序就直接连（ADR-0023 实测）。
pub const PROXY_VARS: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"];

const NO_PROXY_VARS: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// Node 22.21、24.5 起有这个开关，不设就不读代理变量。
const NODE_SWITCH: (&str, &str) = ("NODE_USE_ENV_PROXY", "1");

/// 不走代理的地址，放在 `NO_PROXY` 的两头：中间是核心的别名（`.wonton.internal`，含身份提供方）和本插件的服务名。
/// `*.localhost` 是基础域名，出口容器不认，也没有 DNS 解它。
const NO_PROXY_LOOPBACK: &str = "localhost,.localhost,127.0.0.1,::1,.wonton.internal";
/// 私有网段只写 IPv4。IPv6 网段（`fc00::/7`、`fe80::/10`）带冒号，httpx 把 NO_PROXY 条目转成挂载键时给
/// IPv6 加方括号，`fc00::/7` 变成 `all://[fc00::/7]`，建客户端解析这个键就报 `InvalidURL: Invalid port: ':'`
/// 崩掉（Immich 机器学习下载模型就是这样 500 的，httpx 0.28.1 实测），也没有一种所有客户端都认的 IPv6
/// 网段写法（`[::1]` 在 curl 里反而不匹配）。删掉后这些网段的目标发给出口容器，被它的拒绝名单挡回 403，
/// 和本来就不认网段的 httpx、aiohttp、Node、wget 一样（curl、requests、Go 从直连变成 403）。
const NO_PROXY_PRIVATE: &str = ".local,.lan,.home.arpa,10.0.0.0/8,100.64.0.0/10,169.254.0.0/16,\
                                172.16.0.0/12,192.168.0.0/16";

/// 主人配了代理时，注入给出网插件的内容。没配代理时没有它，插件和没有这个功能时一样直连。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressVars {
    /// 宿主网络的服务用的出口地址，如 `http://10.213.0.125:3128`。宿主网络里解析不了 [`PROXY_URL`] 的别名，
    /// 只能写出口容器在插件网络里的地址（固定，不随容器重建变）；插件没有宿主网络的服务时为空。
    pub host_proxy: Option<String>,
}

impl EgressVars {
    fn no_proxy<'a>(&self, services: impl Iterator<Item = &'a str>) -> String {
        let mut parts = vec![NO_PROXY_LOOPBACK];
        parts.extend(services.map(|service| -> &str { service }));
        parts.push(NO_PROXY_PRIVATE);
        parts.join(",")
    }

    /// 给容器写上代理变量。`services` 是本插件所有的服务名。compose 里写了同名代理变量时以 compose 为准，写成空就是
    /// 这个服务不走代理；`NO_PROXY`/`no_proxy` 不覆盖，把核心名单里缺的条目并进去——作者写的名单常常只有它自己的
    /// 服务，内部请求会被送去上游代理，上游解析不了插件的服务名（#601）。
    pub fn inject<'a>(
        &self,
        body: &mut ContainerCreateBody,
        services: impl Iterator<Item = &'a str>,
        url: &str,
    ) {
        let env = body.env.get_or_insert_with(Vec::new);
        let written: BTreeSet<String> = env
            .iter()
            .filter_map(|line| line.split_once('='))
            .map(|(name, _)| name.to_owned())
            .collect();
        let no_proxy = self.no_proxy(services);
        let pairs = PROXY_VARS
            .iter()
            .map(|name| (*name, url))
            .chain(NO_PROXY_VARS.iter().map(|name| (*name, no_proxy.as_str())))
            .chain([NODE_SWITCH]);
        for (name, value) in pairs {
            if written.contains(name) {
                if NO_PROXY_VARS.contains(&name) {
                    merge_no_proxy(env, name, value);
                }
            } else {
                env.push(format!("{name}={value}"));
            }
        }
    }
}

/// 插件自己写了 `NO_PROXY`（或小写）：保留它的条目，把核心名单里没有的补到后面，不覆盖。
fn merge_no_proxy(env: &mut [String], name: &str, core: &str) {
    let Some(index) = env
        .iter()
        .position(|line| line.split_once('=').is_some_and(|(n, _)| n == name))
    else {
        return;
    };
    let merged = {
        let existing = env[index]
            .split_once('=')
            .map(|(_, value)| value)
            .unwrap_or_default();
        let mut entries: Vec<&str> = existing
            .split(',')
            .map(|entry| entry.trim())
            .filter(|entry| !entry.is_empty())
            .collect();
        for entry in core.split(',') {
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        format!("{name}={}", entries.join(","))
    };
    env[index] = merged;
}

/// 这行环境变量是写了具体地址的代理变量：只要值不是空的，也不是经核心注入的 [`PROXY_URL`]，就是作者写死的代理。
pub fn hardcoded_proxy(line: &str) -> bool {
    line.split_once('=').is_some_and(|(name, value)| {
        PROXY_VARS.contains(&name) && !value.is_empty() && value != PROXY_URL
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> EgressVars {
        EgressVars { host_proxy: None }
    }

    fn injected(env: &[&str]) -> Vec<String> {
        let mut body = ContainerCreateBody {
            env: Some(env.iter().map(|s| (*s).to_owned()).collect()),
            ..Default::default()
        };
        vars().inject(&mut body, ["web", "db"].into_iter(), super::PROXY_URL);
        body.env.unwrap()
    }

    fn value<'a>(env: &'a [String], name: &str) -> Option<&'a str> {
        env.iter().find_map(|line| {
            let (n, v) = line.split_once('=')?;
            (n == name).then_some(v)
        })
    }

    #[test]
    fn both_cases_and_the_node_switch_are_written() {
        let env = injected(&[]);
        for name in PROXY_VARS {
            assert_eq!(value(&env, name), Some(PROXY_URL), "{name}");
        }
        assert_eq!(value(&env, "NODE_USE_ENV_PROXY"), Some("1"));
        let expected = "localhost,.localhost,127.0.0.1,::1,.wonton.internal,web,db,\
                        .local,.lan,.home.arpa,10.0.0.0/8,100.64.0.0/10,169.254.0.0/16,\
                        172.16.0.0/12,192.168.0.0/16";
        assert_eq!(value(&env, "NO_PROXY"), Some(expected));
        assert_eq!(value(&env, "no_proxy"), Some(expected));
    }

    #[test]
    fn no_proxy_has_no_ipv6_cidr_entries() {
        // httpx 把 NO_PROXY 条目转成挂载键，IPv6 加方括号：`fc00::/7` 变成 `all://[fc00::/7]`，
        // 建客户端解析就崩（httpx 0.28.1 实测）。NO_PROXY 里带前缀长度的条目只能是 IPv4。
        let all = format!("{NO_PROXY_LOOPBACK},{NO_PROXY_PRIVATE}");
        for entry in all.split(',') {
            if let Some((addr, _)) = entry.split_once('/') {
                assert!(
                    addr.parse::<std::net::Ipv4Addr>().is_ok(),
                    "{entry} 不是 IPv4 网段，httpx 会崩"
                );
            }
        }
    }

    #[test]
    fn what_compose_wrote_wins_for_proxy_vars() {
        let env = injected(&["HTTP_PROXY=", "KEEP=1"]);
        assert_eq!(value(&env, "HTTP_PROXY"), Some(""));
        assert_eq!(value(&env, "HTTPS_PROXY"), Some(PROXY_URL));
        assert_eq!(value(&env, "KEEP"), Some("1"));
        assert_eq!(
            env.iter().filter(|l| l.starts_with("HTTP_PROXY=")).count(),
            1
        );
    }

    #[test]
    fn compose_written_no_proxy_is_merged_not_replaced() {
        // 作者写的名单常常只有它自己的服务：并上核心的条目，内部请求才不会被送去上游代理（#601）。
        let expected = format!("only-this,{}", full_no_proxy());
        let env = injected(&["no_proxy=only-this"]);
        assert_eq!(value(&env, "no_proxy"), Some(expected.as_str()));
        // 大小写两份独立合并，没写的照常完整注入。
        assert_eq!(value(&env, "NO_PROXY"), Some(full_no_proxy().as_str()));
    }

    #[test]
    fn merged_no_proxy_keeps_entries_and_deduplicates() {
        let env = injected(&["NO_PROXY=web,localhost, 127.0.0.1 ,"]);
        assert_eq!(
            value(&env, "NO_PROXY"),
            Some(
                "web,localhost,127.0.0.1,.localhost,::1,.wonton.internal,db,\
                 .local,.lan,.home.arpa,10.0.0.0/8,100.64.0.0/10,169.254.0.0/16,\
                 172.16.0.0/12,192.168.0.0/16"
            )
        );
        // 写成空就是这个服务没写名单：照常给全。
        let env = injected(&["no_proxy="]);
        assert_eq!(value(&env, "no_proxy"), Some(full_no_proxy().as_str()));
    }

    fn full_no_proxy() -> String {
        vars().no_proxy(["web", "db"].into_iter())
    }

    #[test]
    fn only_concrete_proxy_addresses_count_as_hardcoded() {
        assert!(hardcoded_proxy("HTTP_PROXY=http://172.17.0.1:7899"));
        assert!(hardcoded_proxy("https_proxy=http://proxy.example:3128"));
        assert!(!hardcoded_proxy("HTTP_PROXY="));
        assert!(!hardcoded_proxy(&format!("http_proxy={PROXY_URL}")));
        assert!(!hardcoded_proxy("NO_PROXY=localhost"));
    }
}
