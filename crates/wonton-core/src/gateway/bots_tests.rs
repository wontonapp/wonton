//! Bot 的入口按令牌认调用方（ADR-0045）：共享 Bot 的接口只给主人；Bot 的 Matrix 入口只放行这个插件自己的
//! Bot，别的插件的令牌被拒绝。

use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::fakes::loopback_tunnel;
use super::tests::{TOKEN, owner};
use super::{Gateway, router};
use crate::config::{PLUGIN_MATRIX_HOST, PublicUrls};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::start;
use crate::owner::OwnerStore;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginId, PluginManager};
use wonton_protocol::BotKeeper;

struct Home {
    addr: SocketAddr,
    gateway: Arc<Gateway>,
    matrix: Arc<Matrix>,
    _dir: tempfile::TempDir,
}

async fn start_home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let owner_ids = std::fs::metadata(dir.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: "wonton.localhost".into(),
        port: addr.port(),
    };
    let profile = OwnerStore::for_test(dir.path(), &owner());
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        relay_dir: dir.path().join("relay"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner_ids.uid(),
        pgid: owner_ids.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: addr.port(),
        plugin_slice: None,
        instance: Default::default(),
    };
    let manager = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    let (_fake, config) = start().await;
    let matrix = Arc::new(
        Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap(),
    );
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    let gateway = Arc::new(
        Gateway::new(
            public,
            profile,
            TOKEN.into(),
            members,
            manager,
            None,
            HttpClient::new(),
            tunnel,
        )
        .with_matrix(Some(matrix.clone())),
    );
    let app = router(gateway.clone());
    tokio::spawn(async move {
        let service = app.into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, service).await.unwrap();
    });
    Home {
        addr,
        gateway,
        matrix,
        _dir: dir,
    }
}

impl Home {
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .resolve("wonton.localhost", self.addr)
            .resolve(PLUGIN_MATRIX_HOST, self.addr)
            .build()
            .unwrap()
    }

    async fn send(
        &self,
        url: &str,
        method: Method,
        headers: &[(&str, String)],
        body: &str,
    ) -> (StatusCode, Value) {
        let mut request = self.client().request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
            url,
        );
        for (name, value) in headers {
            request = request.header(*name, value.clone());
        }
        if !body.is_empty() {
            request = request.body(body.to_owned());
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        (hyper::StatusCode::from_u16(status.as_u16()).unwrap(), value)
    }

    fn core_url(&self, path: &str) -> String {
        format!("http://wonton.localhost:{}{path}", self.addr.port())
    }

    fn matrix_url(&self, path: &str) -> String {
        format!("http://{PLUGIN_MATRIX_HOST}:{}{path}", self.addr.port())
    }
}

#[tokio::test]
async fn shared_bots_only_answer_to_the_owner() {
    let home = start_home().await;
    let url = home.core_url("/_wonton/api/bots");
    let body = json!({"plugin_id": "app.echo", "bot": "family"}).to_string();
    // 没有令牌：401。
    let (status, _) = home.send(&url, Method::POST, &[], &body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // 主人令牌，插件没装：404。
    let owner_auth: Vec<(&str, String)> = vec![("Authorization", format!("Bearer {TOKEN}"))];
    let (status, _) = home.send(&url, Method::POST, &owner_auth, &body).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // 请求体不是建 Bot 的参数：400。
    let (status, _) = home.send(&url, Method::POST, &owner_auth, "{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_bot_can_only_use_its_own_plugins_matrix_entry() {
    let home = start_home().await;
    let echo = PluginId::parse("app.echo").unwrap();
    // 两个插件各自的 Bot 都登录了设备；来源是 app.echo 的网络。
    let echo_token = home.matrix.login("echo", "Wonton 插件").await.unwrap();
    let other_token = home.matrix.login("other", "Wonton 插件").await.unwrap();
    home.gateway.plugins.register_source_for_test(
        "app.echo",
        "127.0.0.0/8",
        vec![Permission::MatrixBot],
    );
    home.gateway.plugins.bot_records_for_test(
        &echo,
        &[(
            "default",
            "@echo:test",
            BotKeeper::User {
                username: "owner".to_owned(),
            },
            "bus-token",
        )],
    );
    // 自己的 Bot：请求照常转发给 Tuwunel。
    let own: Vec<(&str, String)> = vec![(
        "Authorization",
        format!("Bearer {}", echo_token.access_token),
    )];
    let (status, body) = home
        .send(
            &home.matrix_url("/_matrix/client/v3/account/whoami"),
            Method::GET,
            &own,
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user_id"], "@echo:test");
    // 别的插件的 Bot 的令牌：403。
    let foreign: Vec<(&str, String)> = vec![(
        "Authorization",
        format!("Bearer {}", other_token.access_token),
    )];
    let (status, body) = home
        .send(
            &home.matrix_url("/_matrix/client/v3/account/whoami"),
            Method::GET,
            &foreign,
            "",
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["errcode"], "M_FORBIDDEN");
    // 认不出的令牌：403。
    let unknown: Vec<(&str, String)> = vec![("Authorization", "Bearer nope".to_owned())];
    let (status, _) = home
        .send(
            &home.matrix_url("/_matrix/client/v3/account/whoami"),
            Method::GET,
            &unknown,
            "",
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
