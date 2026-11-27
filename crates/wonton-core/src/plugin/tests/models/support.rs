//! 共用夹具：只认签发凭据的假网关、模型网关的提供方清单、使用方清单，以及假 Docker 上的 Bed。

use super::*;

pub(crate) type Keys = Arc<Mutex<BTreeMap<String, String>>>;

/// 假网关：`GET /v1/models` 只认签发过的凭据。
pub(crate) async fn fake_gateway(keys: Keys) -> u16 {
    let app = Router::new().route(
        "/v1/models",
        get(move |headers: HeaderMap| {
            let keys = keys.clone();
            async move {
                let bearer = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .unwrap_or_default()
                    .to_owned();
                if keys.lock().unwrap().values().any(|k| *k == bearer) {
                    (StatusCode::OK, r#"{"data":[]}"#)
                } else {
                    (
                        StatusCode::UNAUTHORIZED,
                        r#"{"error":{"message":"unauthorized"}}"#,
                    )
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

pub(crate) fn provider(port: u16) -> PluginPackage {
    let source = format!(
        "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.4'\n  id: system.models\n  version: 1.0.0\n  \
         kind: system\n  title: {{zh: 模型, en: Models}}\n  description: {{zh: 模型, en: Models}}\n  \
         provides:\n    models:\n      service: magpie\n      port: {port}\n      \
         apis: {{openai: /v1, anthropic: /}}\n      default_model: group/default\n      credentials:\n        \
         issue: [sh, /wonton/credentials.sh, issue, '{{name}}']\n        \
         revoke: [sh, /wonton/credentials.sh, revoke, '{{name}}']\n  permissions: []"
    );
    PluginPackage::parse(&source, &super::langs()).unwrap()
}

/// 一个使用方：唯一的服务拿到模型网关的全部变量。
pub(crate) fn consumer() -> String {
    package(
        "  web:\n    image: nginx\n    environment:\n      BASE: ${WONTON_MODEL_BASE_URL}\n      \
         ANTHROPIC: ${WONTON_MODEL_ANTHROPIC_BASE_URL}\n      KEY: ${WONTON_MODEL_API_KEY}\n      \
         MODEL: ${WONTON_MODEL_DEFAULT}",
        "  permissions: [models]",
    )
}

pub(crate) fn models() -> Vec<super::super::Permission> {
    vec!["models".parse().unwrap()]
}

pub(crate) struct Bed {
    pub(crate) docker: FakeDocker,
    pub(crate) keys: Keys,
    pub(crate) manager: Arc<PluginManager>,
    pub(crate) _dir: tempfile::TempDir,
}

/// 装好假网关（预装系统插件的路径），凭据命令由假 Docker 按 magpie 脚本的约定回应。
pub(crate) async fn bed() -> Bed {
    let bed = bed_without_gateway().await;
    bed.install_gateway().await;
    bed
}

/// magpie 脚本的约定（`credentials.sh`）：`issue` 已有同名凭据时不改动，`revoke` 本来没有也算成功。
/// 失败注入把处理函数换掉之后，用它换回真的：issue 成功必须把值写进假网关的 `keys`。
pub(crate) fn credential_script(
    keys: Keys,
) -> impl Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync + 'static {
    move |_, cmd, stdin| {
        let mut keys = keys.lock().unwrap();
        let value = String::from_utf8_lossy(stdin).trim().to_owned();
        match cmd {
            [_, _, action, name] if action == "issue" => {
                keys.entry(name.clone()).or_insert(value);
                (0, String::new())
            }
            [_, _, action, name] if action == "revoke" => {
                keys.remove(name);
                (0, String::new())
            }
            _ => (2, "用法不对".to_owned()),
        }
    }
}

/// 同 [`bed`]，但没有模型网关：声明 `models` 的插件就装在“没接上”的状态。
pub(crate) async fn bed_without_gateway() -> Bed {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Keys::default();
    docker.on_exec(credential_script(keys.clone()));
    let dir = tempfile::tempdir().unwrap();
    // 有 Bot 的使用方还要 Matrix。
    let (_tuwunel, config) = crate::matrix::fake::start().await;
    let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
        .await
        .unwrap();
    let manager = Arc::new(manager_with(engine, &dir).with_matrix(Some(Arc::new(matrix))));
    Bed {
        docker,
        keys,
        manager,
        _dir: dir,
    }
}

impl Bed {
    /// 装上假网关：装完核心就给没接上的使用方签发凭据（见安装完成的那条钩子）。
    pub(crate) async fn install_gateway(&self) {
        let port = fake_gateway(self.keys.clone()).await;
        self.manager
            .install_checked(
                provider(port),
                &[],
                &[],
                false,
                &[],
                Default::default(),
                &super::langs(),
            )
            .await
            .unwrap();
    }

    /// 使用方详情页的轻提示。
    pub(crate) async fn hints(&self) -> wonton_protocol::PluginDetail {
        self.manager
            .judged_detail(&PluginId::parse("app.test").unwrap(), &zh())
            .await
            .unwrap()
    }
}

pub(crate) fn web_env(bed: &Bed) -> BTreeMap<String, String> {
    env_of_container(&bed.docker, &container_name("app-test", "web"))
}

/// 针对某个凭据名执行过的命令数（签发或吊销）；网关自己的 health 凭据不算。
pub(crate) fn credential_execs(bed: &Bed, name: &str) -> usize {
    bed.docker
        .execs()
        .iter()
        .filter(|exec| exec.cmd.get(3).map(String::as_str) == Some(name))
        .count()
}
