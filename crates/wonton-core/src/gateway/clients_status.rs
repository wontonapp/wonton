//! 公网访问总开关和入口状态（ADR-0090 第 2、7 节）：挂在 [`super::clients_api`] 的中间件后面，只给主人。
//!
//! 给不出的字段如实空着或标 `Unknown`，不编造：自检还没跑过，封禁和待完成的事情还没有生产者，
//! 访客链接和入口设置的路由也不在这里。

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use hyper::HeaderMap;
use wonton_protocol::{
    CaaState, CertificateState, ClientsFailure, ClientsStatus, DomainSettings, GateState,
    IngressKind, IngressSettings, LanRecordState, ProbeResult, PublicAccess, PublicItem,
    PublicReach, PublicSwitchState,
};

use super::Gateway;
use super::api::languages;
use super::clients_api::{failure, no_store, unavailable};
use crate::entry_access::EntryAdmin;
use crate::entry_settings::EntrySettings;
use crate::l10n;
use crate::plugin::languages::Languages;

pub fn routes() -> axum::Router<Arc<Gateway>> {
    axum::Router::new()
        .route(&wonton_protocol::clients_status_path(), get(status))
        .route(&wonton_protocol::clients_public_access_path(), put(switch))
}

async fn status(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let langs = languages(&headers);
    let Some(entry) = &gateway.entry else {
        return unavailable(&langs);
    };
    no_store(Json(view(&gateway, entry, None, &langs).await).into_response())
}

/// `PUT`：改总开关，回新的状态。同一个值重复下发不动入口。
async fn switch(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<PublicAccess>,
) -> Response {
    let langs = languages(&headers);
    let Some(entry) = &gateway.entry else {
        return unavailable(&langs);
    };
    let gates = match entry.apply(request.enabled, &langs).await {
        Ok(gates) => gates,
        Err(error) => {
            return failure(
                ClientsFailure::Other,
                &l10n::text(
                    &langs,
                    "clients-public-access-save-failed",
                    &[("error", error.to_string().into())],
                ),
            );
        }
    };
    no_store(Json(view(&gateway, entry, Some(gates), &langs).await).into_response())
}

async fn view(
    gateway: &Gateway,
    entry: &EntryAdmin,
    applied: Option<PublicSwitchState>,
    langs: &Languages,
) -> ClientsStatus {
    let settings = entry.settings().ok().flatten();
    // “不要名字”的装法没有入口设置文件：主人在 App 里配的自己的域名（`entry/domain.json`）就是入口和公网名。
    let stored = match &settings {
        Some(_) => None,
        None => crate::clients::domain::stored_suffix(entry.install_dir())
            .ok()
            .flatten(),
    };
    let enabled = entry.access.enabled();
    let gates = match applied {
        Some(gates) => gates,
        None => gates(entry, enabled, langs).await,
    };
    // 有设置文件时公网名是家名服务的名字；没有设置文件时配的域名本身就是公网名。
    let public_name = match &settings {
        Some(_) => gateway.matrix.as_ref().is_some_and(|matrix| {
            !crate::matrix::federation_config::is_private(matrix.server_name())
        }),
        None => stored.is_some(),
    };
    let ingress = ingress(entry.install_dir(), settings.as_ref(), stored.as_deref());
    ClientsStatus {
        core_in_container: entry.core_in_container(),
        domain: domain(gateway).await,
        public_reach: reach(enabled, public_name, &gates),
        ingress,
        public_access: PublicAccess { enabled },
        gates,
        self_probe: ProbeResult::NotRun,
        public_now: public_now(gateway, enabled),
        bans: Vec::new(),
        pending: Vec::new(),
    }
}

async fn domain(gateway: &Gateway) -> DomainSettings {
    let empty = || DomainSettings {
        suffix: None,
        suffix_editable: true,
        home: None,
        tail: None,
        provider: None,
        credentials_set: false,
        terms_accepted: false,
        certificate: CertificateState::None,
        caa: CaaState::Unknown,
        caa_record: None,
        lan_record: LanRecordState::Unknown,
        lan_target: None,
    };
    let Some(domain) = &gateway.domain else {
        return empty();
    };
    match domain.view().await {
        Ok(view) => view,
        Err(error) => {
            tracing::warn!(%error, "域名设置读不了");
            DomainSettings {
                certificate: CertificateState::Other,
                ..empty()
            }
        }
    }
}

/// 公网怎么进来：家名注册用 frpc（地址和令牌在 `frpc.json`）；没有注册但配了域名就是路由器转发的发布端口，
/// 域名来自设置文件（有设置文件时）或 App 里配的自己的域名（没有设置文件时）。
fn ingress(
    install_dir: &Path,
    settings: Option<&EntrySettings>,
    stored: Option<&str>,
) -> IngressSettings {
    let mut ingress = IngressSettings {
        kind: None,
        port: None,
        server_addr: None,
        server_port: None,
        server_name: None,
        ca_pem: None,
        token_set: false,
    };
    let entry = install_dir.join("entry");
    if settings.is_some_and(|settings| settings.registration.is_some()) {
        ingress.kind = Some(IngressKind::OwnFrps);
        if let Ok(text) = std::fs::read_to_string(entry.join("frpc.json"))
            && let Ok(config) = serde_json::from_str::<serde_json::Value>(&text)
        {
            ingress.server_addr = config["serverAddr"].as_str().map(str::to_owned);
            ingress.server_port = config["serverPort"]
                .as_u64()
                .and_then(|port| port.try_into().ok());
            ingress.server_name = config["transport"]["tls"]["serverName"]
                .as_str()
                .map(str::to_owned);
            ingress.token_set = config["metadatas"]["token"].as_str().is_some();
        }
        ingress.ca_pem = std::fs::read_to_string(entry.join("relay-ca.pem")).ok();
    } else if settings.is_some() || stored.is_some() {
        ingress.kind = Some(IngressKind::RouterForward);
        ingress.port = std::env::var("WONTON_ENTRY_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .or(Some(8443));
    }
    ingress
}

async fn gates(entry: &EntryAdmin, enabled: bool, langs: &Languages) -> PublicSwitchState {
    let Some(containers) = entry.containers() else {
        return PublicSwitchState {
            connector: GateState::Unknown,
            crt_list: GateState::Unknown,
            core: GateState::Unknown,
        };
    };
    let connector = match containers.connector_running().await {
        Ok(running) if running == enabled => GateState::Done,
        Ok(_) => GateState::Failed {
            detail: l10n::text(langs, "clients-gate-connector-mismatch", &[]),
        },
        Err(error) => GateState::Failed {
            detail: error.to_string(),
        },
    };
    let crt_list = match containers.public_crt_list_empty() {
        Ok(empty) if empty != enabled => GateState::Done,
        Ok(_) => GateState::Failed {
            detail: l10n::text(langs, "clients-gate-public-list-mismatch", &[]),
        },
        Err(error) => GateState::Failed {
            detail: error.to_string(),
        },
    };
    PublicSwitchState {
        connector,
        crt_list,
        core: GateState::Done,
    }
}

/// 有公网名才有公网可达性：有设置文件时看家名服务的名字，没有设置文件时看 App 里配的域名。
fn reach(enabled: bool, public_name: bool, gates: &PublicSwitchState) -> PublicReach {
    if !enabled {
        return PublicReach::PublicAccessOff;
    }
    if !public_name {
        return PublicReach::NoDomain;
    }
    if matches!(gates.connector, GateState::Failed { .. }) {
        return PublicReach::ConnectorDown;
    }
    // 重试和自检还没接上：开着但没有成功过，如实说不知道。
    PublicReach::Unknown
}

fn public_now(gateway: &Gateway, enabled: bool) -> Vec<PublicItem> {
    if !enabled {
        return Vec::new();
    }
    let Ok(plugins) = gateway.plugins.client_plugins() else {
        return Vec::new();
    };
    plugins
        .into_iter()
        .filter(|client| client.record.clients_entries.public)
        .filter_map(|client| {
            Some(PublicItem {
                rules: client.public_rules.clone(),
                plugin_id: client.id.to_string(),
                label: client.record.entry_label.clone()?,
                guest_links: Vec::new(),
            })
        })
        .collect()
}
