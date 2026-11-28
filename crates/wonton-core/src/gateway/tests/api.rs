//! 插件 API 要主人令牌，并按请求的语言回答。

use serde_json::json;

use super::*;

#[tokio::test]
async fn plugin_api_requires_owner_token() {
    let h = Harness::start(None).await;
    let denied = h.get(CORE, "/_wonton/api/plugins", &[]).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let cookie_only = h
        .get(CORE, "/_wonton/api/plugins", &[("cookie", &core)])
        .await;
    assert_eq!(
        cookie_only.status,
        StatusCode::UNAUTHORIZED,
        "Cookie 不能调用 API"
    );
    let bearer = format!("Bearer {TOKEN}");
    let listed = h
        .get(CORE, "/_wonton/api/plugins", &[("authorization", &bearer)])
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json()["plugins"][0]["id"], "app.whoami");
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/whoami");
    let package = PluginPackage::load_dir(&dir, &Languages::from_header(Some("zh-Hans"))).unwrap();
    let preview = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[
                ("authorization", &bearer),
                ("content-type", "application/x-tar"),
            ],
            Body::from(package.archive()),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    let preview = preview.json();
    let blocked = preview["blocked"].as_str().unwrap();
    // 请求没带 Accept-Language，回退英文；期望值用目录本身格式化出来。
    let expected = crate::l10n::text(
        &crate::plugin::languages::Languages::default(),
        "install-already-installed",
        &[
            ("id", "app.whoami".into()),
            ("version", package.manifest.version.as_str().into()),
        ],
    );
    assert_eq!(blocked, expected);
    assert!(preview["digest"].as_str().unwrap().starts_with("sha256:"));
    let bad = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[("authorization", &bearer)],
            Body::from("x: ["),
        )
        .await;
    assert_eq!(bad.error_kind(), Some("invalid_input"));

    // 引用包内配置的上传必须带完整插件包，只有 compose 文本时应拒绝。
    let compose = "services:\n  app:\n    image: nginx\n    configs: [settings]\nconfigs:\n  settings:\n    file: config.yaml\nx-wonton:\n  spec: '0.4'\n  id: app.config-upload\n  version: 1.0.0\n  kind: app\n  title: {zh: 配置, en: Config}\n  description: {zh: 配置上传, en: Config upload}\n  permissions: []\n";
    let files = [("config.yaml".to_owned(), b"enabled: true\n".to_vec())].into();
    let package = PluginPackage::with_files(compose, files, &Languages::default()).unwrap();
    let archive = package.archive();
    let tar = [
        ("authorization", bearer.as_str()),
        ("content-type", "application/x-tar"),
    ];
    let preview = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &tar,
            Body::from(archive),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json()["id"], "app.config-upload");
    // 只有 compose 文本时，引用的 config.yaml 缺失。
    let text = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[("authorization", &bearer)],
            Body::from(compose),
        )
        .await;
    assert_eq!(text.error_kind(), Some("invalid_input"));
}

#[tokio::test]
async fn plugin_api_answers_in_the_requested_language() {
    let h = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let english = [
        ("authorization", bearer.as_str()),
        ("accept-language", "en-GB, zh;q=0.5"),
    ];
    let listed = h.get(CORE, "/_wonton/api/plugins", &english).await.json();
    let whoami = &listed["plugins"][0];
    assert_eq!(whoami["title"], "Who am I");
    assert_eq!(
        whoami["status"],
        json!({"state": "running", "health": "unknown"})
    );
    assert_eq!(whoami["web_url"], format!("{}/", h.origin(PLUGIN)));
    assert_eq!(whoami["trust"], "local");

    let detail = h
        .get(CORE, "/_wonton/api/plugins/app.whoami", &english)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text);
    assert_eq!(detail.json()["installed"]["id"], "app.whoami");
    let missing = h
        .get(CORE, "/_wonton/api/plugins/app.missing", &english)
        .await;
    assert_eq!(missing.error_kind(), Some("not_installed"));
    let image = format!("/_wonton/api/images/{}/icon-192.png", "0".repeat(64));
    assert_eq!(
        h.get(CORE, &image, &english).await.status,
        StatusCode::NOT_FOUND
    );

    // JSON 请求体是按来源预览和安装，不当成 compose 文本去解析：来源写错、少了摘要都是请求不对。
    let json = [
        ("authorization", bearer.as_str()),
        ("content-type", "application/json"),
    ];
    for path in ["/_wonton/api/plugins/preview", "/_wonton/api/plugins"] {
        let body = Body::from(r#"{"source": "github.com/example/plugin"}"#);
        let reply = h.send(Method::POST, CORE, path, &json, body).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(reply.error_kind(), Some("invalid_input"), "{path}");
    }
}
