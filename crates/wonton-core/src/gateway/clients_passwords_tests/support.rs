//! 夹具：清单常量、凭据执行替身、临时目录上的 Home 和 Gateway。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use hyper::Method;
use hyper::header::AUTHORIZATION;
use hyper::header::CONTENT_TYPE;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::clients::credentials::{CredentialExec, ExecError, ExecOutput};
use crate::config::PublicUrls;
use crate::gateway::fakes::loopback_tunnel;
use crate::gateway::tests::{CORE, Reply, TOKEN, owner};
use crate::gateway::{Gateway, TunnelPeer, router};
use crate::http::HttpClient;
use crate::mcp::audit::Audit;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::plugin::PluginPackage;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use futures_util::future::BoxFuture;

pub(super) const PLUGIN: &str = "app.demo.gw";

pub(super) const MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.gw
  version: 1.0.0
  kind: app
  title: { zh: 凭据应用, en: Credentials app }
  description:
    zh: 凭据接口测试用
    en: For password API tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{user}"
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          revoke: [pw, revoke, "{name}"]
          ends_sessions: false
"#;

/// 共享密码的清单（`per_client: false`）：吊销默认换值，`keep_shared_passwords` 时原样留着。
pub(super) const KEEP_SHARED_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.gw
  version: 1.0.0
  kind: app
  title: { zh: 凭据应用, en: Credentials app }
  description:
    zh: 凭据接口测试用
    en: For password API tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{user}"
          per_client: false
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          rotate: [pw, rotate, "{name}"]
          ends_sessions: false
"#;

/// 网关测试不关心应用里的输出；退出 0 就当命令成功。
pub(super) struct AlwaysOk;

/// 固定一种结果，测 502 和超时。
#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Exit(i64),
    Timeout,
}

pub(super) struct Fixed(pub(super) Outcome);

impl CredentialExec for Fixed {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        _argv: &'a [String],
        _stdin: Option<&'a [u8]>,
        _timeout: std::time::Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        Box::pin(async {
            match self.0 {
                Outcome::Exit(code) => Ok(ExecOutput {
                    exit_code: code,
                    stdout: String::new(),
                    stderr: "应用拒绝".into(),
                }),
                Outcome::Timeout => Err(ExecError::Timeout),
            }
        })
    }
}

impl CredentialExec for AlwaysOk {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        _argv: &'a [String],
        _stdin: Option<&'a [u8]>,
        _timeout: std::time::Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        Box::pin(async {
            Ok(ExecOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

/// 数跑过几次命令：keep_shared 吊销一次都不该跑。
#[derive(Clone, Default)]
pub(super) struct Counting(Arc<AtomicUsize>);

impl Counting {
    pub(super) fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl CredentialExec for Counting {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        _argv: &'a [String],
        _stdin: Option<&'a [u8]>,
        _timeout: std::time::Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ExecOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

pub(super) struct Home {
    pub(super) app: Router,
    pub(super) dir: tempfile::TempDir,
    pub(super) gateway: Arc<Gateway>,
    pub(super) manifest: &'static str,
}

pub(super) async fn home() -> Home {
    home_with(MANIFEST).await
}

pub(super) async fn home_with(manifest: &'static str) -> Home {
    home_with_manifest_and_file(manifest, None).await
}

pub(super) async fn home_with_password_file(contents: Option<&str>) -> Home {
    home_with_manifest_and_file(GATEWAY_MANIFEST, contents).await
}

pub(super) async fn home_with_manifest_and_file(
    manifest: &'static str,
    contents: Option<&str>,
) -> Home {
    let dir = tempfile::tempdir().unwrap();
    if let Some(contents) = contents {
        let path = dir.path().join(crate::clients::passwords::FILE_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 18480,
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
        puid: 1000,
        pgid: 1000,
        install_timeout: std::time::Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
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
    manager.set_credential_exec(Arc::new(AlwaysOk));
    let parsed = PluginPackage::parse(manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    manager.register_for_test(
        parsed,
        crate::plugin::manager::PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
    manager.clients_entry_for_test(
        &crate::plugin::PluginId::parse(PLUGIN).unwrap(),
        "gw",
        false,
        false,
    );
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    let audit = Audit::new(dir.path().join("audit.jsonl"));
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
        .with_audit(Some(Arc::new(audit))),
    );
    Home {
        app: router(gateway.clone()),
        dir,
        gateway,
        manifest,
    }
}

impl Home {
    pub(super) async fn send(&self, request: Request<Body>) -> Reply {
        Reply::of(self.app.clone().oneshot(request).await.unwrap()).await
    }

    pub(super) fn request(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("{CORE}:18480"))
            .header("x-wonton-clients", "1")
    }

    pub(super) fn owner_request(
        &self,
        method: Method,
        path: &str,
    ) -> hyper::http::request::Builder {
        self.request(method, path)
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
    }

    pub(super) async fn as_owner(
        &self,
        device: iroh::EndpointId,
        method: Method,
        path: &str,
        body: &str,
    ) -> Reply {
        self.send(
            self.request(method, path)
                .extension(TunnelPeer(device))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
    }

    pub(super) fn audit_lines(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.dir.path().join("audit.jsonl")).unwrap();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub(super) fn set_status(&self, status: crate::plugin::manager::PluginStatus) {
        let manifest =
            PluginPackage::parse(self.manifest, &Languages::from_header(Some("zh-Hans")))
                .unwrap()
                .manifest;
        self.gateway
            .manager()
            .register_for_test(manifest, status, Some("127.0.0.1:9".into()));
    }

    pub(super) async fn create(&self, label: &str) -> Reply {
        self.send(
            self.owner_request(
                Method::POST,
                &format!("/_wonton/clients/{PLUGIN}/passwords"),
            )
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "label": label }).to_string()))
            .unwrap(),
        )
        .await
    }
}

impl Gateway {
    pub(super) fn manager(&self) -> &Arc<PluginManager> {
        &self.plugins
    }
}

pub(super) const GATEWAY_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.gw
  version: 1.0.0
  kind: app
  title: { zh: 网关应用, en: Gateway app }
  description:
    zh: 网关验证测试用
    en: For gateway auth tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        gateway:
          headers:
            X-Remote-User: "{user}"
"#;
