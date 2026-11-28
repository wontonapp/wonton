//! App 经隧道管理核心：插件列表、设置、升级都认设备身份；插件网页上带过来的请求不算 App。

use super::*;

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use super::super::tests::PLUGIN;

/// ADR-0017：App 经隧道管理插件；插件网页的请求同样带着设备身份，读也要拒绝。
#[tokio::test]
async fn paired_devices_manage_plugins_but_web_pages_on_them_cannot() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let path = "/_wonton/api/plugins";

    let (status, _, text) = phone.send(Method::GET, CORE, path, &[]).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(
        serde_json::from_str::<wonton_protocol::InstalledPlugins>(&text).is_ok(),
        "{text}"
    );
    let same_origin = format!("http://{CORE}:{PORT}");
    for page in [("origin", same_origin.as_str()), ("sec-fetch-site", "none")] {
        let (status, _, text) = phone.send(Method::GET, CORE, path, &[page]).await;
        assert_eq!(status, StatusCode::OK, "{page:?}: {text}");
    }

    // 出网代理的设置页和网页局域网端口的开关都在 App 里（ADR-0023）。没带请求体，过了鉴权才回 400。
    let (status, _, text) = phone
        .send(Method::GET, CORE, "/_wonton/api/egress-proxy", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    for route in [
        "/_wonton/api/egress-proxy",
        "/_wonton/api/plugins/app.whoami/lan-webui",
    ] {
        let (status, _, text) = phone.send(Method::PUT, CORE, route, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{route}: {text}");
    }
    // 升级也在 App 里。过了鉴权才看请求：空的请求体不是合法的上传包，预览时这个插件没装。
    let upgrade = "/_wonton/api/plugins/app.whoami/upgrade";
    let (status, _, text) = phone.send(Method::POST, CORE, upgrade, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let preview = "/_wonton/api/plugins/app.whoami/upgrade/preview";
    let (status, headers, text) = phone.send(Method::POST, CORE, preview, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert_eq!(header(&headers, "x-wonton-error"), Some("not_installed"));

    // 升级也在 App 里（ADR-0025）。这个核心没接上升级，过了鉴权才回 503。
    let (status, _, text) = phone
        .send(Method::GET, CORE, "/_wonton/api/core/upgrade", &[])
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{text}");

    let plugin_page = format!("http://{PLUGIN}:{PORT}");
    for (method, route) in [
        (Method::GET, path),
        (Method::POST, path),
        (Method::POST, "/_wonton/api/plugins/preview"),
        (Method::GET, "/_wonton/api/plugins/app.whoami"),
        (Method::DELETE, "/_wonton/api/plugins/app.whoami"),
        (Method::GET, "/_wonton/api/images/abc/icon-192.png"),
        (Method::GET, "/_wonton/api/store"),
        (
            Method::GET,
            "/_wonton/api/store/plugin?source=github:owner/repo",
        ),
        (Method::POST, "/_wonton/api/store/sources"),
        (Method::DELETE, "/_wonton/api/store/sources?address=x"),
        (Method::GET, "/_wonton/api/egress-proxy"),
        (Method::PUT, "/_wonton/api/egress-proxy"),
        (Method::GET, "/_wonton/api/core/upgrade"),
        (Method::POST, "/_wonton/api/core/upgrade"),
        (Method::DELETE, "/_wonton/api/core/upgrade"),
        (Method::POST, "/_wonton/api/core/upgrade/check"),
        (Method::PUT, "/_wonton/api/plugins/app.whoami/lan-webui"),
        (Method::POST, "/_wonton/api/plugins/app.whoami/upgrade"),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade/preview",
        ),
    ] {
        for page in [
            ("origin", plugin_page.as_str()),
            ("origin", "null"),
            ("sec-fetch-site", "same-site"),
            ("sec-fetch-site", "cross-site"),
        ] {
            let (status, headers, _) = phone.send(method.clone(), CORE, route, &[page]).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {route} {page:?}");
            assert_eq!(header(&headers, "x-wonton-error"), Some("cross_site"));
        }
    }

    // 设备列表认主人的设备（ADR-0045）；家人的设备会拿到 403 permission_required（family_tests）。
    // 局域网设置在这里读不了：Home 的引擎连不上 Docker，LAN 探测如实报 503，鉴权在 family_tests 的表里。
    let (status, _, _) = phone
        .send(Method::GET, CORE, "/_wonton/api/devices", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "主人的设备能读");
    // App 连上后跟着服务器的地址走，插件网页不能借设备身份探测。
    let route = "/_wonton/api/tunnel";
    let (status, _, text) = phone.send(Method::GET, CORE, route, &[]).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let page = ("origin", plugin_page.as_str());
    let (status, _, _) = phone.send(Method::GET, CORE, route, &[page]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = lan(&home, Method::GET, CORE, path).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "局域网请求没有设备身份");
}
