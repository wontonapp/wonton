//! `sso: none` 的插件靠应用自带的登录：主人第一次打开时在应用里注册管理员（ADR-0022）。首次注册的安全全靠网关挡在前面。

use std::sync::atomic::Ordering;

use axum::body::Body;
use hyper::{Method, StatusCode};

use super::tests::{Harness, PLUGIN};
use crate::plugin::languages::Languages;
use crate::plugin::manifest::Sso;
use crate::plugin::package::PluginPackage;

async fn register(h: &Harness, headers: &[(&str, &str)]) -> StatusCode {
    h.send(Method::POST, PLUGIN, "/setup", headers, Body::from("admin"))
        .await
        .status
}

#[tokio::test]
async fn only_the_owner_reaches_the_registration_page_of_an_sso_none_plugin() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/whoami");
    let manifest = PluginPackage::load_dir(&dir, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    assert_eq!(manifest.webui.unwrap().sso, Sso::None);
    let h = Harness::start(None).await;

    // 没登录的：局域网里任何一台设备。
    let page = h.get(PLUGIN, "/setup", &[("accept", "text/html")]).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "打开页面先去核心登录");
    assert_eq!(register(&h, &[]).await, StatusCode::UNAUTHORIZED);
    let forged = [("cookie", "wonton_app=forged")];
    assert_eq!(register(&h, &forged).await, StatusCode::UNAUTHORIZED);
    assert_eq!(h.hits.load(Ordering::SeqCst), 0, "注册请求不得到达插件");

    // 别的插件的页面：借着主人的会话也提交不了注册。
    let session = h.app_session().await;
    let other = h.origin("app-evil.wonton.localhost");
    let cross_site = [("cookie", session.as_str()), ("origin", other.as_str())];
    assert_eq!(register(&h, &cross_site).await, StatusCode::UNAUTHORIZED);
    let same_site = [
        ("cookie", session.as_str()),
        ("sec-fetch-site", "same-site"),
    ];
    assert_eq!(register(&h, &same_site).await, StatusCode::UNAUTHORIZED);
    assert_eq!(h.hits.load(Ordering::SeqCst), 0, "注册请求不得到达插件");

    // 主人自己。
    let own = [
        ("cookie", session.as_str()),
        ("sec-fetch-site", "same-origin"),
    ];
    assert_eq!(register(&h, &own).await, StatusCode::OK);
    assert_eq!(h.hits.load(Ordering::SeqCst), 1);
}
