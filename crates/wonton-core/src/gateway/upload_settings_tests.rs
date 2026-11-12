//! 上传安装、升级带设置值：值走 `Wonton-Plugin-Settings` 头（base64url JSON），写法同 JSON 安装请求的 `settings`。

use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::tests::{CORE, Harness, Reply};
use crate::plugin::languages::Languages;
use crate::plugin::names::container_name;
use crate::plugin::package_files::PackageFiles;
use crate::plugin::{PluginId, PluginPackage};

const ID: &str = "app.consent";

/// `eula` 必填、没有默认值：不带值装不上。核心检查清单时按默认值试渲染，没有默认值的变量要在 compose 里写回退。
const SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["eula"],
  "properties": {
    "eula": { "type": "boolean", "x-wonton-title": { "en": "Accept the EULA" } },
    "players": {
      "type": "integer",
      "minimum": 1,
      "maximum": 100,
      "default": 20,
      "x-wonton-title": { "en": "Max players" }
    }
  }
}"#;

fn archive(version: &str) -> Vec<u8> {
    let compose = format!(
        "services:\n  game:\n    image: nginx\n    environment:\n      EULA: '${{WONTON_SETTING_EULA:-false}}'\n      \
         PLAYERS: '${{WONTON_SETTING_PLAYERS}}'\nx-wonton:\n  spec: '0.13'\n  id: {ID}\n  version: {version}\n  \
         kind: app\n  title: {{en: Consent}}\n  description: {{en: Consent}}\n  permissions: []\n  \
         settings: settings.schema.json\n"
    );
    let files: PackageFiles = [(
        "settings.schema.json".to_owned(),
        SCHEMA.as_bytes().to_vec(),
    )]
    .into();
    PluginPackage::with_files(&compose, files, &Languages::default())
        .unwrap()
        .archive()
}

fn header(values: serde_json::Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&values).unwrap())
}

async fn upload(h: &Harness, path: &str, body: Vec<u8>, settings: Option<&str>) -> Reply {
    let bearer = format!("Bearer {}", super::tests::TOKEN);
    let mut headers = vec![
        ("authorization", bearer.as_str()),
        ("content-type", "application/x-tar"),
    ];
    if let Some(settings) = settings {
        headers.push(("wonton-plugin-settings", settings));
    }
    h.send(Method::POST, CORE, path, &headers, Body::from(body))
        .await
}

#[tokio::test]
async fn uploads_take_settings_from_the_header() {
    let h = Harness::start(None).await;
    let id = PluginId::parse(ID).unwrap();
    let plain = upload(&h, "/_wonton/api/plugins?grant=", archive("1.0.0"), None).await;
    assert_eq!(plain.status, StatusCode::BAD_REQUEST, "{}", plain.text);
    assert_eq!(plain.error_kind(), Some("settings_invalid"));
    assert!(h.manager.record_for_test(&id).is_none());

    let eula = header(json!({"eula": {"type": "flag", "value": true}}));
    let installed = upload(
        &h,
        "/_wonton/api/plugins?grant=",
        archive("1.0.0"),
        Some(&eula),
    )
    .await;
    assert_eq!(installed.status, StatusCode::CREATED, "{}", installed.text);
    let record = h.manager.record_for_test(&id).unwrap();
    assert_eq!(record.settings["eula"], json!(true));
    assert_eq!(record.settings["players"], json!(20));
    let game = container_name("app-consent", "game");
    assert!(h.docker.env(&game).iter().any(|e| e == "EULA=true"));

    // 升级只带改动的项，旧值留着。
    let players = header(json!({"players": {"type": "integer", "value": 5}}));
    let path = format!("/_wonton/api/plugins/{ID}/upgrade?grant=");
    let upgraded = upload(&h, &path, archive("1.1.0"), Some(&players)).await;
    assert_eq!(upgraded.status, StatusCode::OK, "{}", upgraded.text);
    let record = h.manager.record_for_test(&id).unwrap();
    assert_eq!(record.settings["eula"], json!(true));
    assert_eq!(record.settings["players"], json!(5));
}

#[tokio::test]
async fn a_broken_settings_header_is_invalid_input() {
    let h = Harness::start(None).await;
    let reply = upload(
        &h,
        "/_wonton/api/plugins?grant=",
        archive("1.0.0"),
        Some("!!!not-base64"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_kind(), Some("invalid_input"));
    assert!(
        reply.text.contains("Wonton-Plugin-Settings"),
        "{}",
        reply.text
    );
}
