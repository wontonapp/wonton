//! 插件密钥：清单规则，以及在假 Docker 上的生成、注入、保留、收存和回滚。

use std::fs;

use super::super::PluginPackage;
use super::super::fake_docker::{FakeDocker, FakeFile};
use super::super::manifest::PluginId;
use super::super::names::container_name;
use super::super::package::PackageError;
use super::super::store::Store;
use super::manager::manager_with;
use super::package;

/// 一个服务，环境变量里引用核心生成的 `web_key`；`extra` 追加在 `secrets` 下。
fn with_secrets(extra: &str) -> String {
    package(
        "  web:\n    image: nginx\n    environment:\n      KEY: ${WONTON_SECRET_WEB_KEY}",
        &format!(
            "  permissions: []\n  secrets:\n    web_key:\n      generate: {{bytes: 16}}\n{extra}"
        ),
    )
}

const RECOVERY: &str = "    recovery:\n      collect: {service: web, path: /data/recovery.txt}\n";

fn env_value(env: &[String], key: &str) -> Option<String> {
    env.iter()
        .find_map(|e| e.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

#[test]
fn secret_declarations_are_checked() {
    for bad in [
        "    Web:\n      generate: {}\n",
        "    two:\n      generate: {}\n      collect: {service: web, path: /a}\n",
        "    none: {}\n",
        "    big:\n      generate: {bytes: 4096}\n",
        "    rel:\n      collect: {service: web, path: data/a}\n",
        "    up:\n      collect: {service: web, path: /data/../etc/shadow}\n",
        "    enc:\n      generate: {encoding: base32}\n",
    ] {
        assert!(
            matches!(
                PluginPackage::parse(&with_secrets(bad), &super::langs()),
                Err(PackageError::Manifest(_))
            ),
            "{bad}"
        );
    }
    let unknown = with_secrets(&RECOVERY.replace("service: web", "service: nope"));
    assert!(matches!(
        PluginPackage::parse(&unknown, &super::langs()),
        Err(PackageError::UnknownService { .. })
    ));
    // 没声明的密钥变量和别的未定义变量一样，安装前就报错。
    let undeclared = with_secrets("").replace("WONTON_SECRET_WEB_KEY", "WONTON_SECRET_OTHER");
    let pkg = PluginPackage::parse(&undeclared, &super::langs()).unwrap();
    assert!(super::super::render::dry_run(&pkg, &super::langs()).is_err());
}

#[tokio::test]
async fn generated_secrets_are_injected_and_kept_until_data_is_removed() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let id = PluginId::parse("app.test").unwrap();
    let web = container_name("app-test", "web");
    m.install(&with_secrets(""), &[], super::langs())
        .await
        .unwrap();
    let key = env_value(&docker.env(&web), "KEY").expect("注入了密钥");
    assert!(
        key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()),
        "16 字节的十六进制：{key}"
    );
    let file = dir.path().join("plugins/secrets/app.test.json");
    let mode = fs::metadata(&file).unwrap().permissions();
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
        0o600
    );

    // 不删数据卸载后重装：沿用原来的值，已经写进数据里的密码才对得上。
    m.uninstall(&id, false, &super::langs()).await.unwrap();
    assert!(file.exists(), "不删数据时密钥随数据保留");
    m.install(&with_secrets(""), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(env_value(&docker.env(&web), "KEY").as_deref(), Some(&*key));

    m.uninstall(&id, true, &super::langs()).await.unwrap();
    assert!(!file.exists(), "删除数据时密钥一起删除");
    m.install(&with_secrets(""), &[], super::langs())
        .await
        .unwrap();
    assert_ne!(env_value(&docker.env(&web), "KEY"), Some(key));
}

#[tokio::test]
async fn secrets_written_by_the_plugin_are_collected_and_kept_up_to_date() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);
    let id = PluginId::parse("app.test").unwrap();
    let web = container_name("app-test", "web");
    let stored = || {
        let store = Store::open(dir.path().join("plugins")).unwrap();
        store.secrets().load(&id).unwrap().unwrap_or_default()
    };
    // 插件还没写下文件：照常装好，之后再收。
    m.install(&with_secrets(RECOVERY), &[], super::langs())
        .await
        .unwrap();
    assert!(!stored().contains_key("recovery"));

    let file = |text: &str| Some(FakeFile::Regular(text.as_bytes().to_vec()));
    docker.set_file(&web, "/data/recovery.txt", file("key-1"));
    let restarted = manager_with(engine.clone(), &dir);
    restarted.restore().await.unwrap();
    assert_eq!(stored()["recovery"], b"key-1");
    // 每次启动核对一次，变了就更新；收存的值不注入回插件。
    docker.set_file(&web, "/data/recovery.txt", file("key-2"));
    manager_with(engine, &dir).restore().await.unwrap();
    assert_eq!(stored()["recovery"], b"key-2");
    assert!(env_value(&docker.env(&web), "WONTON_SECRET_RECOVERY").is_none());
}

#[tokio::test]
async fn a_failed_install_leaves_the_kept_secrets_as_they_were() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let id = PluginId::parse("app.test").unwrap();
    // 上次不删数据卸载时保留的密钥，里面还没有这个版本新增的 `token`。
    let store = Store::open(dir.path().join("plugins")).unwrap();
    let kept = [("web_key".to_owned(), b"kept".to_vec())].into();
    store.secrets().save(&id, &kept).unwrap();
    // 要收存的路径是个目录：只收普通文件，安装失败并回滚。
    let web = container_name("app-test", "web");
    docker.set_file(&web, "/data/recovery.txt", Some(FakeFile::Directory));
    let compose = with_secrets(&format!("{RECOVERY}    token:\n      generate: {{}}\n"));
    let err = m.install(&compose, &[], super::langs()).await.unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert_eq!(store.secrets().load(&id).unwrap(), Some(kept));
}

/// 主人填写的一项必填密钥和一项可选密钥，服务里各引用一个变量。
fn with_input(required: bool) -> String {
    package(
        "  web:\n    image: nginx\n    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}\n      GREETING: ${WONTON_SECRET_GREETING}",
        &format!(
            "  permissions: []\n  secrets:\n    token:\n      input:\n        title: {{en: Token}}\n        required: {required}\n    greeting:\n      input:\n        title: {{en: Greeting}}\n        required: false\n"
        ),
    )
    .replace("spec: '0.5'", "spec: '0.8'")
}

#[tokio::test]
async fn input_values_are_injected_and_optional_ones_are_empty() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let id = PluginId::parse("app.test").unwrap();
    let store = Store::open(dir.path().join("plugins")).unwrap();
    store
        .secrets()
        .save(
            &id,
            &[("token".to_owned(), b"from-the-owner".to_vec())].into(),
        )
        .unwrap();
    m.install(&with_input(true), &[], super::langs())
        .await
        .unwrap();
    let web = container_name("app-test", "web");
    assert_eq!(
        env_value(&docker.env(&web), "TOKEN").as_deref(),
        Some("from-the-owner")
    );
    assert_eq!(
        env_value(&docker.env(&web), "GREETING").as_deref(),
        Some(""),
        "可选没填时注入空串"
    );
}

/// 必填的密钥没填：照样装上，状态是“待配置”，MCP 请求头先空着；填上后状态恢复，头和环境变量都换成新值。
#[tokio::test]
async fn an_unfilled_required_input_waits_for_configuration_instead_of_blocking() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = std::sync::Arc::new(manager_with(engine, &dir));
    let id = PluginId::parse("app.test").unwrap();
    let source = package(
        "  web:\n    image: nginx\n    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}",
        "  permissions: []\n  \
         mcp: {service: web, port: 8080, transport: streamable-http, headers: {Authorization: 'Bearer {secret:token}'}}\n  \
         secrets:\n    token:\n      input:\n        title: {en: Token}\n",
    );
    m.install(&source, &[], super::langs()).await.unwrap();
    let web = container_name("app-test", "web");
    assert_eq!(
        env_value(&docker.env(&web), "TOKEN").as_deref(),
        Some(""),
        "没填的必填项注入空串"
    );
    let authorization = || {
        m.mcp_servers()
            .into_iter()
            .find(|server| server.id == id)
            .expect("装上了就有 MCP 目标")
            .headers
            .custom_headers()
            .get(&hyper::header::HeaderName::from_static("authorization"))
            .map(|value| value.to_str().unwrap().to_owned())
    };
    assert_eq!(authorization(), None, "没填时头先空着");
    let installed = m.installed_plugin(&id, &super::langs()).unwrap();
    assert_eq!(
        installed.status,
        wonton_protocol::PluginState::NeedsConfiguration
    );

    m.set_secret_input(&id, "token", Some("s3cret"), &super::langs())
        .await
        .unwrap();
    assert_eq!(authorization().as_deref(), Some("Bearer s3cret"));
    assert_eq!(
        env_value(&docker.env(&web), "TOKEN").as_deref(),
        Some("s3cret")
    );
    let installed = m.installed_plugin(&id, &super::langs()).unwrap();
    assert!(matches!(
        installed.status,
        wonton_protocol::PluginState::Running { .. }
    ));
}

#[tokio::test]
async fn the_detail_shows_filled_and_time_but_never_the_value() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let id = PluginId::parse("app.test").unwrap();
    let store = Store::open(dir.path().join("plugins")).unwrap();
    let secrets = [("token".to_owned(), b"s3cret".to_vec())].into();
    let changed = [("token".to_owned(), 1_700_000_000_000u64)].into();
    store
        .secrets()
        .save_changed(&id, &secrets, &changed)
        .unwrap();
    m.install(&with_input(true), &[], super::langs())
        .await
        .unwrap();
    let detail = m.judged_detail(&id, &super::langs()).await.unwrap();
    let token = detail
        .secret_inputs
        .iter()
        .find(|line| line.name == "token")
        .unwrap();
    assert!(token.filled);
    assert_eq!(token.changed_at_ms, Some(1_700_000_000_000));
    let json = serde_json::to_string(&detail).unwrap();
    assert!(!json.contains("s3cret"), "{json}");
}
