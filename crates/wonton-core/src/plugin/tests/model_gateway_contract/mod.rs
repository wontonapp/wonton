//! The same consumer contract runs against unrelated gateway implementations.
//! Only adapters know provider packaging; requests use the injected URL, key and opaque model ID.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use wonton_protocol::{CustomModelProvider, SecretString};

use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::model_providers::ProviderAction;
use crate::plugin::models::ModelsRoute;
use crate::plugin::names::Instance;
use crate::plugin::provides::ModelsProvider;
use crate::plugin::{PluginId, PluginManager, PluginPackage, PluginStatus};
use crate::test_docker::Names;

use super::manager::manager_in_instance;
use super::{env_of_container, langs, package};

mod consumer;
mod fake;
mod management;
mod response_upstream;
mod responses;
mod system;
mod upstream;

pub(super) struct Target {
    pub spec: ModelsProvider,
    pub upstream: String,
    pub backend_url: String,
    pub docker: FakeDocker,
    pub engine: Engine,
    pub _guard: Box<dyn Any + Send + Sync>,
}

struct Bed {
    manager: Arc<PluginManager>,
    target: Target,
    http: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    instance: Instance,
    _names: Names,
    _dir: tempfile::TempDir,
}

impl Bed {
    async fn new(target: Target) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut names = Names::new();
        let instance = names.instance().clone();
        let mut manager = manager_in_instance(target.engine.clone(), &dir, names.name());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        manager.cfg.gateway_port = endpoint.port();
        let (_matrix_server, config) = crate::matrix::fake::start().await;
        let matrix = crate::matrix::Matrix::connect(
            &config,
            &dir.path().join("core"),
            crate::http::HttpClient::new(),
        )
        .await
        .unwrap();
        manager = manager.with_matrix(Some(Arc::new(matrix)));
        let mut manifest = PluginPackage::parse(
            &package("  gateway:\n    image: test-only", "  permissions: []")
                .replace("id: app.test", "id: com.example.gateway")
                .replace("kind: app", "kind: system"),
            &langs(),
        )
        .unwrap()
        .manifest;
        manifest.provides = Some(crate::plugin::provides::Provides {
            models: Some(target.spec.clone()),
            egress_proxy: None,
            capabilities: BTreeMap::new(),
        });
        let provider = names.oneshot_container("contract-provider");
        target
            .docker
            .add_container(&provider, "com.example.gateway");
        let route = ModelsRoute {
            id: manifest.id.clone(),
            spec: target.spec.clone(),
            upstream: target.upstream.clone(),
            credentials_container: provider,
        };
        manager.register_for_test(manifest, PluginStatus::Running, None);
        manager.serve_models_for_test(route);
        manager.register_source_for_test(
            "com.example.consumer",
            "127.0.0.0/8",
            vec!["models".parse().unwrap()],
        );
        let manager = Arc::new(manager);
        let (tunnel, members) = crate::gateway::fakes::loopback_tunnel(dir.path()).await;
        let gateway = crate::gateway::Gateway::new(
            manager.cfg.public.clone(),
            manager.cfg.profile.clone(),
            "contract-owner".into(),
            members,
            manager.clone(),
            None,
            crate::http::HttpClient::new(),
            tunnel,
        );
        let router = crate::gateway::router(Arc::new(gateway));
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let http = reqwest::Client::builder()
            .no_proxy()
            .resolve("models.wonton.internal", endpoint)
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        Self {
            manager,
            target,
            http,
            server,
            instance,
            _names: names,
            _dir: dir,
        }
    }

    fn custom(&self, key: &str) -> CustomModelProvider {
        CustomModelProvider {
            base_url: self.target.backend_url.clone(),
            model: "contract-text".into(),
            key: SecretString::new(key),
        }
    }

    async fn configured(&self) -> Vec<String> {
        let listed = self.manager.model_providers(&langs()).await.unwrap();
        assert!(listed.custom);
        assert_eq!(listed.any_configured, !listed.configured.is_empty());
        listed.configured.into_iter().map(|p| p.id).collect()
    }

    async fn install(&self, id: &str, bot: bool) -> BTreeMap<String, String> {
        let mut environment = String::from(
            "  client:\n    image: test-only\n    environment:\n      WONTON_MODEL_BASE_URL: ${WONTON_MODEL_BASE_URL}\n      WONTON_MODEL_API_KEY: ${WONTON_MODEL_API_KEY}\n      WONTON_MODEL_DEFAULT: ${WONTON_MODEL_DEFAULT}",
        );
        for (present, name) in [
            (
                self.target.spec.apis.anthropic.is_some(),
                "WONTON_MODEL_ANTHROPIC_BASE_URL",
            ),
            (
                self.target.spec.apis.gemini.is_some(),
                "WONTON_MODEL_GEMINI_BASE_URL",
            ),
        ] {
            if present {
                environment.push_str(&format!("\n      {name}: ${{{name}}}"));
            }
        }
        let manifest = if bot {
            "  bot: {display_name: {en: Contract}}\n  bots: {services: [client]}\n  permissions: [matrix.bot, models]"
        } else {
            "  permissions: [models]"
        };
        let grants = if bot {
            vec!["matrix.bot".parse().unwrap(), "models".parse().unwrap()]
        } else {
            vec!["models".parse().unwrap()]
        };
        let source = package(&environment, manifest).replace("id: app.test", &format!("id: {id}"));
        self.manager
            .install(&source, &grants, langs())
            .await
            .unwrap();
        let slug = PluginId::parse(id).unwrap().slug();
        let name = if bot {
            self.instance.bot_container(&slug, "client", "default")
        } else {
            self.instance.container(&slug, "client")
        };
        env_of_container(&self.target.docker, &name)
    }

    async fn chat(&self, env: &BTreeMap<String, String>, body: Value) -> reqwest::Response {
        self.http
            .post(format!("{}/chat/completions", env["WONTON_MODEL_BASE_URL"]))
            .bearer_auth(&env["WONTON_MODEL_API_KEY"])
            .json(&body)
            .send()
            .await
            .unwrap()
    }
}

impl Drop for Bed {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn contract(target: Target, upstream: &upstream::Upstream) {
    let bed = Bed::new(target).await;
    management::configure(&bed).await;
    let first = bed.install("com.example.first", false).await;
    let second = bed.install("com.example.second", false).await;
    let bot = bed.install("com.example.bot", true).await;
    assert_ne!(bot["WONTON_MODEL_API_KEY"], first["WONTON_MODEL_API_KEY"]);
    assert_ne!(bot["WONTON_MODEL_API_KEY"], second["WONTON_MODEL_API_KEY"]);
    consumer::catalog(&bed, &bot).await;
    assert_ne!(
        first["WONTON_MODEL_API_KEY"],
        second["WONTON_MODEL_API_KEY"]
    );
    consumer::exercise(&bed, &first, upstream).await;
    responses::exercise(&bed, &first).await;
    assert!(bed.manager.reconcile_models().await.is_empty());
    consumer::catalog(&bed, &first).await;
    bed.manager
        .uninstall(
            &PluginId::parse("com.example.first").unwrap(),
            true,
            &langs(),
        )
        .await
        .unwrap();
    let response = bed
        .http
        .get(format!("{}/models", first["WONTON_MODEL_BASE_URL"]))
        .bearer_auth(&first["WONTON_MODEL_API_KEY"])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    consumer::catalog(&bed, &second).await;
    management::finish(&bed, &second).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_minimal_gateway_satisfies_the_consumer_contract() {
    let upstream = Arc::new(upstream::Upstream::start().await);
    contract(fake::start(upstream.port).await, &upstream).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要 Docker，锁定的系统网关运行与最小网关相同的消费方契约"]
async fn the_bundled_gateway_satisfies_the_same_consumer_contract() {
    let upstream = Arc::new(upstream::Upstream::start().await);
    contract(system::start(&upstream.socket).await, &upstream).await;
}
