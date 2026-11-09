//! 核心升级改了注入的值以后，已装插件的容器按新渲染重建（ADR-0063）：渲染不变不动，缺标签的旧容器重建一次，
//! 卷和数据目录留着，重建失败照实报。

use std::sync::{Arc, Mutex};

use axum::routing::{get, put};
use axum::{Json, Router};
use hyper::StatusCode;
use serde_json::json;

use super::super::PluginStatus;
use super::super::fake_docker::FakeDocker;
use super::egress::serve;
use super::manager::manager_with_oidc;
use super::package;
use crate::config::OidcProviderConfig;
use crate::http::HttpClient;
use crate::oidc::Hydra;

/// 签发方可以中途换掉的假身份提供方：模拟核心升级换了 `WONTON_OIDC_ISSUER`。
async fn fake_hydra(issuer: Arc<Mutex<String>>) -> Hydra {
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let issuer = issuer.lock().unwrap().clone();
                async move {
                    Json(json!({
                        "issuer": issuer,
                        "authorization_endpoint": "http://auth.wonton.localhost:8080/oauth2/auth",
                        "token_endpoint": "http://auth.wonton.internal:8080/oauth2/token",
                        "userinfo_endpoint": "http://auth.wonton.internal:8080/userinfo",
                        "jwks_uri": "http://auth.wonton.internal:8080/.well-known/jwks.json",
                        "revocation_endpoint": "http://auth.wonton.internal:8080/oauth2/revoke",
                        "end_session_endpoint": "http://auth.wonton.localhost:8080/oauth2/sessions/logout",
                    }))
                }
            }),
        )
        .route("/admin/clients/{id}", put(|| async { StatusCode::OK }));
    let addr = serve(app).await;
    Hydra::new(
        HttpClient::new(),
        &OidcProviderConfig {
            admin_url: format!("http://{addr}"),
            public_url: format!("http://{addr}"),
        },
    )
}

/// 带名字的卷、应用数据目录和 SSO 的插件。port 是清单的网页入口端口：异步测试传 [`webui`] 起的服务，
/// 只解析不安装的测试不连它，任意合法值都行。
fn app(port: u16) -> String {
    let services = "  web:\n    image: nginx\n    environment:\n      WONTON_OIDC_ISSUER: '${WONTON_OIDC_ISSUER}'\n    \
                    volumes: ['data:/data', '${WONTON_APP_DATA}/data:/data']";
    format!(
        "volumes:\n  data: {{}}\n{}",
        package(
            services,
            &format!(
                "  permissions: []\n  webui:\n    service: web\n    port: {port}\n    path: /\n    sso: oidc\n    \
                 oidc:\n      redirect_paths: [/oidc/callback]"
            ),
        )
    )
}

/// 假 Docker 把插件容器的地址报成 127.0.0.1，安装最后检查的网页入口就是这个本机服务：
/// 和 egress 的安装一样，测试自己 serve，不靠宿主碰巧在监听某个端口。
async fn webui() -> u16 {
    serve(Router::new().route("/", get(|| async { "ok" })))
        .await
        .port()
}

/// 插件的容器和它用的卷。
const WEB: &str = "wonton-app-test--web";
const VOLUME: &str = "wonton-app-test--data";

fn created_env(docker: &FakeDocker, name: &str) -> Vec<String> {
    docker.created(name).unwrap()["Env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect()
}

fn issuer_of(docker: &FakeDocker, name: &str) -> String {
    created_env(docker, name)
        .into_iter()
        .find_map(|entry| entry.strip_prefix("WONTON_OIDC_ISSUER=").map(str::to_owned))
        .expect("容器里有签发方变量")
}

fn data_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("apps/app-test/data/keep.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "存着的数据").unwrap();
    path
}

#[tokio::test]
async fn an_unchanged_render_restores_without_recreating() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hydra = fake_hydra(Arc::new(Mutex::new("http://auth.wonton.internal".into()))).await;
    let manager = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    manager
        .install(&app(webui().await), &[], super::langs())
        .await
        .unwrap();
    let kept = data_file(&dir);

    drop(manager);
    let restarted = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    restarted.restore().await.unwrap();

    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.creations(WEB), 1);
    assert!(kept.exists());
}

#[tokio::test]
async fn a_changed_issuer_recreates_the_containers_and_keeps_the_data() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let issuer = Arc::new(Mutex::new("http://auth.wonton.internal".into()));
    let hydra = fake_hydra(issuer.clone()).await;
    let manager = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    manager
        .install(&app(webui().await), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(issuer_of(&docker, WEB), "http://auth.wonton.internal");
    let kept = data_file(&dir);

    // 升级后的核心按新的签发方渲染。
    *issuer.lock().unwrap() = "http://auth.wonton.internal:9443".into();
    drop(manager);
    let upgraded = manager_with_oidc(engine, &dir, hydra);
    upgraded.restore().await.unwrap();

    assert_eq!(upgraded.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.creations(WEB), 2, "容器要按新渲染重建");
    assert_eq!(issuer_of(&docker, WEB), "http://auth.wonton.internal:9443");
    assert!(kept.exists(), "数据目录留着");
    assert_eq!(docker.volumes(), [VOLUME], "卷留着");
}

#[tokio::test]
async fn a_container_without_the_label_is_recreated_once() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hydra = fake_hydra(Arc::new(Mutex::new("http://auth.wonton.internal".into()))).await;
    let manager = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    manager
        .install(&app(webui().await), &[], super::langs())
        .await
        .unwrap();

    // 旧核心建的容器：有归属，没有渲染摘要。
    let mut body = docker.created(WEB).unwrap();
    body["Labels"]
        .as_object_mut()
        .unwrap()
        .remove("wonton.render");
    docker.add_stranger_container(WEB, body);
    docker.add_container(WEB, "app.test");

    drop(manager);
    let restarted = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.creations(WEB), 2);
    assert!(
        docker.created(WEB).unwrap()["Labels"]["wonton.render"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );

    // 重建过的容器摘要对得上，再启动不再重建。
    drop(restarted);
    let again = manager_with_oidc(engine, &dir, hydra);
    again.restore().await.unwrap();
    assert_eq!(docker.creations(WEB), 2);
}

#[tokio::test]
async fn a_failed_recreate_is_reported_instead_of_using_the_old_container() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let issuer = Arc::new(Mutex::new("http://auth.wonton.internal".into()));
    let hydra = fake_hydra(issuer.clone()).await;
    let manager = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    manager
        .install(&app(webui().await), &[], super::langs())
        .await
        .unwrap();

    *issuer.lock().unwrap() = "http://auth.wonton.internal:9443".into();
    docker.fail_starts(Some("容器起不来"));
    drop(manager);
    let upgraded = manager_with_oidc(engine, &dir, hydra);
    upgraded.restore().await.unwrap();

    let (status, health) = {
        let view = &upgraded.list()[0];
        (view.status.clone(), view.health)
    };
    let (PluginStatus::Degraded { reason }, None) = (status, health) else {
        panic!("重建失败要标成异常")
    };
    assert!(reason.contains("容器起不来"), "{reason}");
    // 旧容器已经删掉，留下的是新渲染出来的那个，只是没起来。
    assert_eq!(docker.creations(WEB), 2);
    assert_eq!(issuer_of(&docker, WEB), "http://auth.wonton.internal:9443");
    assert!(docker.containers().contains(&WEB.to_owned()));
}

#[tokio::test]
async fn a_stopped_plugin_is_recreated_when_started_not_at_restore() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let issuer = Arc::new(Mutex::new("http://auth.wonton.internal".into()));
    let hydra = fake_hydra(issuer.clone()).await;
    let manager = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    let installed = manager
        .install(&app(webui().await), &[], super::langs())
        .await
        .unwrap();
    docker.set_state(
        WEB,
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    docker.set_start_state(
        WEB,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    manager.watch_containers().await;

    *issuer.lock().unwrap() = "http://auth.wonton.internal:9443".into();
    drop(manager);
    let upgraded = std::sync::Arc::new(manager_with_oidc(engine, &dir, hydra));
    upgraded.restore().await.unwrap_or_else(|e| {
        panic!(
            "停着的插件核对不该重建：{e}（创建了 {} 次）",
            docker.creations(WEB)
        )
    });
    assert_eq!(upgraded.list()[0].status, PluginStatus::Stopped);
    assert_eq!(docker.creations(WEB), 1, "停着的插件核对时不动");

    upgraded.start_detached(installed.id).await.unwrap();
    assert_eq!(upgraded.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.creations(WEB), 2);
    assert_eq!(issuer_of(&docker, WEB), "http://auth.wonton.internal:9443");
}

#[test]
fn the_digest_ignores_the_label_itself_and_unordered_fields() {
    let pkg = super::super::PluginPackage::parse(&app(1), &super::langs()).unwrap();
    let mut plan = super::super::render::dry_run(&pkg, &super::langs()).unwrap();
    let before = super::super::render_drift::digest(&plan.containers[0]);
    super::super::render_drift::stamp(&mut plan.containers[0]);
    assert_eq!(
        super::super::render_drift::digest(&plan.containers[0]),
        before
    );
    assert_eq!(
        plan.containers[0].body.labels.as_ref().unwrap()["wonton.render"],
        before
    );
    // 换一个值摘要就不同。
    plan.containers[0].body.image = Some("nginx:1.27".into());
    assert_ne!(
        super::super::render_drift::digest(&plan.containers[0]),
        before
    );
}

#[test]
fn the_digest_covers_the_container_name_and_the_written_files() {
    let pkg = super::super::PluginPackage::parse(&app(1), &super::langs()).unwrap();
    let plan = super::super::render::dry_run(&pkg, &super::langs()).unwrap();
    let original = super::super::render_drift::digest(&plan.containers[0]);

    let mut renamed = plan.containers[0].clone();
    renamed.name.push_str("-2");
    assert_ne!(super::super::render_drift::digest(&renamed), original);

    let mut with_file = plan.containers[0].clone();
    with_file.files.push(super::super::configs::ConfigFile {
        target: "/etc/app.conf".into(),
        uid: 1000,
        gid: 1000,
        mode: 0o444,
        data: b"issuer=old".to_vec(),
    });
    assert_ne!(super::super::render_drift::digest(&with_file), original);
}

#[test]
fn the_same_render_always_hashes_the_same() {
    let pkg = super::super::PluginPackage::parse(&app(1), &super::langs()).unwrap();
    let first = super::super::render::dry_run(&pkg, &super::langs()).unwrap();
    let second = super::super::render::dry_run(&pkg, &super::langs()).unwrap();
    assert_eq!(
        super::super::render_drift::digest(&first.containers[0]),
        super::super::render_drift::digest(&second.containers[0]),
        "标签、端口这些 HashMap 的顺序不能影响摘要"
    );
}
