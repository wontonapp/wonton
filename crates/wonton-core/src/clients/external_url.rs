//! 规范对外地址 `WONTON_EXTERNAL_URL`（ADR-0089 第 5 节）：按入口开关取值、注入给插件，
//! `single_host` 的请求按它改写 `Host`。公网访问总开关不参与计算。
//!
//! 值只来自主人自己打开的入口；公网优先，其次局域网，都没开时是 App 内经隧道的地址。
//! 入口快照（[`crate::clients::state`]）在记录或域名变化时算好存着，请求热路径不读盘。

use crate::clients::state::ClientsEntries;
use crate::config::PublicUrls;
use crate::gateway::Gateway;
use crate::plugin::manifest::PluginId;
use crate::plugin::store::InstalledRecord;

pub const VAR: &str = "WONTON_EXTERNAL_URL";
/// 局域网入口端口（`WONTON_CLIENT_PORT`，ADR-0021）；App 里显示的地址也用它，端口只有这一个来源。
pub const DEFAULT_CLIENT_PORT: u16 = 8483;

/// 局域网入口端口：`WONTON_CLIENT_PORT` 解析失败或为 0 时用 8483。
pub fn lan_port() -> u16 {
    std::env::var("WONTON_CLIENT_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_CLIENT_PORT)
}

/// 按开关的顺序算值。开着的入口缺标签或后缀时按下一档算；三档都没有时和 `WONTON_APP_URL` 相同。
pub fn value(
    entries: ClientsEntries,
    label: Option<&str>,
    suffix: Option<&str>,
    app_url: Option<&str>,
) -> Option<String> {
    if let (true, Some(label), Some(suffix)) = (entries.public, label, suffix) {
        return Some(format!("https://{label}.{suffix}"));
    }
    if let (true, Some(label), Some(suffix)) = (entries.lan, label, suffix) {
        return Some(format!("https://{label}.lan.{suffix}:{}", lan_port()));
    }
    app_url.map(str::to_owned)
}

/// 核心经隧道给这个插件的地址（`WONTON_APP_URL` 的值；没有网页的插件也照样给）。
pub fn app_origin(public: &PublicUrls, id: &PluginId) -> String {
    public.origin(&public.plugin_host(&id.slug()))
}

/// 安装、重建时注入的值：记录可能还没有（全新安装），按全关算。开关热路径不算，见
/// [`crate::clients::state::State`]。
pub async fn for_record(gateway: &Gateway, record: &InstalledRecord) -> Option<String> {
    let suffix = current_suffix(gateway);
    let app_url = app_origin(&gateway.public, &record.id);
    value(
        record.clients_entries,
        record.entry_label.as_deref(),
        suffix.as_deref(),
        Some(&app_url),
    )
}

/// 解析成（协议，主机带端口）；值是核心自己拼的，格式固定。
pub fn split(url: &str) -> Option<(&str, &str)> {
    url.split_once("://")
}

fn current_suffix(gateway: &Gateway) -> Option<String> {
    let domain = gateway.domain.as_ref()?;
    match domain.suffix() {
        Ok(suffix) => suffix,
        Err(error) => {
            tracing::error!(%error, "读不到域名后缀，规范对外地址按下一条算");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "http://app.core.test:8080";

    #[test]
    fn public_wins_over_lan_and_app() {
        let entries = ClientsEntries {
            lan: true,
            public: true,
        };
        assert_eq!(
            value(entries, Some("app"), Some("example.test"), Some(APP)).unwrap(),
            "https://app.example.test"
        );
    }

    #[test]
    fn lan_uses_the_client_port() {
        let entries = ClientsEntries {
            lan: true,
            public: false,
        };
        assert_eq!(
            value(entries, Some("app"), Some("example.test"), Some(APP)).unwrap(),
            format!("https://app.lan.example.test:{}", lan_port())
        );
    }

    #[test]
    fn closed_entries_use_the_app_url() {
        assert_eq!(
            value(
                ClientsEntries::default(),
                Some("app"),
                Some("example.test"),
                Some(APP)
            )
            .unwrap(),
            APP
        );
        assert!(value(ClientsEntries::default(), None, None, None).is_none());
    }

    #[test]
    fn an_open_entry_without_a_suffix_falls_through() {
        let lan_and_public = ClientsEntries {
            lan: true,
            public: true,
        };
        assert_eq!(
            value(lan_and_public, Some("app"), None, Some(APP)).unwrap(),
            APP
        );
    }

    #[test]
    fn split_reads_scheme_and_authority() {
        assert_eq!(
            split("https://app.lan.example.test:8483"),
            Some(("https", "app.lan.example.test:8483"))
        );
    }
}
