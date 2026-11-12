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
                PluginPackage::parse(&with_secrets(bad)),
                Err(PackageError::Manifest(_))
            ),
            "{bad}"
        );
    }
    let unknown = with_secrets(&RECOVERY.replace("service: web", "service: nope"));
    assert!(matches!(
        PluginPackage::parse(&unknown),
        Err(PackageError::UnknownService { .. })
    ));
    // 没声明的密钥变量和别的未定义变量一样，安装前就报错。
    let undeclared = with_secrets("").replace("WONTON_SECRET_WEB_KEY", "WONTON_SECRET_OTHER");
    let pkg = PluginPackage::parse(&undeclared).unwrap();
    assert!(super::super::render::dry_run(&pkg).is_err());
}

#[tokio::test]
async fn generated_secrets_are_injected_and_kept_until_data_is_removed() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let id = PluginId::parse("app.test").unwrap();
    let web = container_name("app-test", "web");
    m.install(&with_secrets(""), &[]).await.unwrap();
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
    m.uninstall(&id, false).await.unwrap();
    assert!(file.exists(), "不删数据时密钥随数据保留");
    m.install(&with_secrets(""), &[]).await.unwrap();
    assert_eq!(env_value(&docker.env(&web), "KEY").as_deref(), Some(&*key));

    m.uninstall(&id, true).await.unwrap();
    assert!(!file.exists(), "删除数据时密钥一起删除");
    m.install(&with_secrets(""), &[]).await.unwrap();
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
    m.install(&with_secrets(RECOVERY), &[]).await.unwrap();
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
    let err = m.install(&compose, &[]).await.unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert_eq!(store.secrets().load(&id).unwrap(), Some(kept));
}
