use super::{discovery, failed, problem};
use iroh::{Endpoint, RelayMap, RelayMode, endpoint::presets};
use std::net::Ipv4Addr;
use url::Url;
use wonton_matrix::{FriendSource, MatrixSession};
use wonton_protocol::{
    GatewayRefusal, MatrixOpenIdToken, ProxySettings, SecretString, ShareFailure, ShareProblem,
    SharedAppRow, SharedAppsSource, SharedAppsState, SharedPlugin,
};
use wonton_tunnel::{
    ProxyCredential,
    sharing::{ShareClient, ShareError, ShareLocalProxy},
};

pub(super) struct Source {
    pub view: SharedAppsSource,
    connection: std::sync::Arc<std::sync::Mutex<Option<Connection>>>,
}

type Connection = (
    ShareClient,
    ShareLocalProxy,
    wonton_tunnel::sharing::ProxyRegistration,
);

impl Source {
    pub fn new(friend: &FriendSource) -> Self {
        Self {
            view: SharedAppsSource {
                owner_user_id: friend.user_id.to_string(),
                owner_display_name: friend.display_name.clone(),
                server_name: friend.user_id.server_name().to_string(),
                state: SharedAppsState::Loading,
                apps: Vec::new(),
            },
            connection: Default::default(),
        }
    }

    pub fn begin(&mut self) {
        self.disconnect();
        self.view.state = SharedAppsState::Loading;
    }

    fn disconnect(&mut self) {
        if let Some((client, _, _)) = self.connection.lock().expect("source lock").take() {
            client.close();
        }
    }

    pub fn fail(&mut self, problem: ShareProblem) {
        self.disconnect();
        self.view.state = failed(problem);
    }

    pub fn client(&self) -> Option<ShareClient> {
        self.connection
            .lock()
            .expect("source lock")
            .as_ref()
            .map(|(client, _, _)| client.clone())
    }

    pub fn snapshot(&self) -> SharedAppsSource {
        let mut view = self.view.clone();
        if matches!(view.state, SharedAppsState::Ready { .. })
            && self.client().is_none_or(|client| client.is_closed())
        {
            view.state = failed(problem(ShareFailure::Unreachable, None));
        }
        view
    }

    pub fn watch_closed(
        &self,
        view: tokio::sync::watch::Sender<wonton_protocol::PluginsViewModel>,
    ) {
        let Some(client) = self.client() else { return };
        let connection = std::sync::Arc::downgrade(&self.connection);
        let expected = self.view.state.clone();
        let owner = self.view.owner_user_id.clone();
        tokio::spawn(async move {
            client.closed().await;
            let Some(connection) = connection.upgrade() else {
                return;
            };
            view.send_modify(|view| {
                if let Some(shared) = &mut view.shared_apps
                    && let Some(source) = shared
                        .sources
                        .iter_mut()
                        .find(|source| source.owner_user_id == owner)
                    && source.state == expected
                {
                    let mut connection = connection.lock().expect("source lock");
                    if !connection.as_ref().is_some_and(|(current, _, _)| {
                        current.connection_id() == client.connection_id()
                    }) {
                        return;
                    }
                    connection.take();
                    source.state = failed(problem(ShareFailure::Unreachable, None));
                }
            });
        });
    }

    // None means the friend does not publish sharing discovery; no error source.
    pub async fn connect(
        &mut self,
        session: &MatrixSession,
        hub: &wonton_tunnel::sharing::ProxyHub,
        chat: &tokio::sync::watch::Sender<wonton_protocol::ChatViewModel>,
    ) -> Option<Result<(), ShareProblem>> {
        let result = self.open(session, hub, chat).await;
        match result {
            Ok(false) => None,
            Ok(true) => Some(Ok(())),
            Err(error) => Some(Err(error)),
        }
    }

    async fn open(
        &mut self,
        session: &MatrixSession,
        hub: &wonton_tunnel::sharing::ProxyHub,
        chat: &tokio::sync::watch::Sender<wonton_protocol::ChatViewModel>,
    ) -> Result<bool, ShareProblem> {
        let http = discovery::http_client().await?;
        let Some((discovery, fresh)) =
            discovery::discover_and_token(&http, &self.view.server_name, async {
                session.fresh_openid_token().await.map_err(|error| {
                    chat.send_modify(|view| {
                        view.problem = Some(wonton_protocol::ChatProblem {
                            reason: crate::chat::failure(&error),
                            message: error.to_string(),
                        })
                    });
                    problem(ShareFailure::Unreachable, None)
                })
            })
            .await?
        else {
            return Ok(false);
        };
        let relays = RelayMap::from_iter(discovery.relays);
        let transport = iroh::endpoint::QuicTransportConfig::builder()
            .enable_segmentation_offload(false)
            .build();
        let endpoint = Endpoint::builder(presets::N0)
            .relay_mode(RelayMode::Custom(relays))
            .transport_config(transport)
            .bind()
            .await
            .map_err(|_| problem(ShareFailure::Unreachable, None))?;
        let token = MatrixOpenIdToken {
            access_token: SecretString::new(fresh.access_token),
            matrix_server_name: fresh.matrix_server_name,
            expires_in: fresh.expires_in,
        };
        let client = ShareClient::connect(endpoint, discovery.target, token)
            .await
            .map_err(transport_problem)?;
        if client.user_id() != session.user_id().as_str() {
            client.close();
            return Err(problem(ShareFailure::Identity, Some(401)));
        }
        let result = self.attach(client.clone(), hub).await;
        if result.is_err() {
            client.close();
        }
        result.map(|()| true)
    }

    pub(super) async fn attach(
        &mut self,
        client: ShareClient,
        hub: &wonton_tunnel::sharing::ProxyHub,
    ) -> Result<(), ShareProblem> {
        let apps = &client.initial_shares().plugins;
        // An empty authenticated list has no page hosts; the proxy still stays source-scoped.
        let domain = proxy_domain(apps, &self.view.server_name)?;
        let credential = ProxyCredential::generate();
        let proxy = ShareLocalProxy::bind(
            client.clone(),
            (Ipv4Addr::LOCALHOST, 0).into(),
            credential.clone(),
            domain.clone(),
        )
        .await
        .map_err(|_| problem(ShareFailure::Unreachable, None))?;
        let registration = hub
            .register(proxy.local_addr(), credential, domain.clone())
            .map_err(|_| problem(ShareFailure::Unreachable, None))?;
        let credential = registration.credential();
        let settings = ProxySettings {
            host: registration.local_addr().ip().to_string(),
            port: registration.local_addr().port(),
            username: credential.username().to_owned(),
            password: credential.password().to_owned(),
            domain,
        };
        self.view.apps = apps
            .iter()
            .cloned()
            .map(|plugin| SharedAppRow {
                plugin,
                problem: None,
            })
            .collect();
        *self.connection.lock().expect("source lock") = Some((client, proxy, registration));
        self.view.state = SharedAppsState::Ready { proxy: settings };
        Ok(())
    }

    pub async fn probe(&mut self, plugin_id: &str) {
        let SharedAppsState::Ready { proxy } = &self.view.state else {
            return;
        };
        let Some(row) = self
            .view
            .apps
            .iter_mut()
            .find(|row| row.plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .proxy(match reqwest::Proxy::all(crate::chat::proxy_url(proxy)) {
                Ok(proxy) => proxy,
                Err(_) => {
                    row.problem = Some(problem(ShareFailure::Other, None));
                    return;
                }
            })
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let result = match client {
            Ok(client) => client.head(&row.plugin.web_url).send().await.ok(),
            Err(_) => None,
        };
        row.problem = match result {
            Some(response) => response_problem(
                response.status().as_u16(),
                response
                    .headers()
                    .get("x-wonton-error")
                    .and_then(|value| value.to_str().ok()),
            ),
            None => Some(problem(ShareFailure::Unreachable, None)),
        };
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn proxy_domain(apps: &[SharedPlugin], server: &str) -> Result<String, ShareProblem> {
    let fail = || problem(ShareFailure::Other, None);
    let mut domain = None;
    for plugin in apps {
        let url = Url::parse(&plugin.web_url).map_err(|_| fail())?;
        let host = url.host_str().ok_or_else(fail)?;
        let (label, base) = host.split_once('.').ok_or_else(fail)?;
        if label != plugin.plugin_id.replace('.', "-")
            || !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(fail());
        }
        if domain.as_deref().is_some_and(|domain| domain != base) {
            return Err(fail());
        }
        domain = Some(base.to_owned());
    }
    domain
        .or_else(|| {
            Url::parse(&format!("https://{server}"))
                .ok()?
                .host_str()
                .map(str::to_owned)
        })
        .ok_or_else(fail)
}

pub(super) fn transport_problem(error: ShareError) -> ShareProblem {
    match error {
        ShareError::Refused(refusal) => {
            let status = if refusal.refusal == GatewayRefusal::ShareIdentity {
                401
            } else {
                403
            };
            let mut result = response_problem(status, Some(refusal.refusal.as_str()))
                .unwrap_or_else(|| problem(ShareFailure::Other, None));
            result.identity = refusal.identity;
            result
        }
        ShareError::Protocol | ShareError::TooLarge => problem(ShareFailure::Other, None),
        _ => problem(ShareFailure::Unreachable, None),
    }
}

pub(super) fn response_problem(status: u16, code: Option<&str>) -> Option<ShareProblem> {
    let reason = match code {
        Some("share_identity") => ShareFailure::Identity,
        Some("share_required") => ShareFailure::Required,
        Some("share_revoked") => ShareFailure::Revoked,
        Some("share_expired") => ShareFailure::Expired,
        Some("plugin_not_ready") => ShareFailure::PluginNotRunning,
        // 探针是 HEAD：不支持 HEAD 的插件回 405/501 不代表页面不可用。
        _ if status == 405 || status == 501 => return None,
        _ if status >= 500 => ShareFailure::Other,
        _ => return None,
    };
    Some(problem(reason, Some(status)))
}
