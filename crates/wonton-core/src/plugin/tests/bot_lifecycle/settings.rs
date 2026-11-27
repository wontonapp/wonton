//! 改 Bot 插件的设置：重建容器，保留 Matrix 身份。

use std::sync::Arc;

use serde_json::json;

use super::{Bed, echo, granted, id};
use crate::plugin::package_files::PackageFiles;

const LEVEL_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "level": {
      "type": "string",
      "default": "quiet",
      "x-wonton-title": { "zh": "音量", "en": "Level" }
    }
  }
}"#;

fn echo_with_setting() -> String {
    echo()
        .replace("spec: '0.5'", "spec: '0.12'")
        .replace(
            "ROOM: ${WONTON_MATRIX_HOME_ROOM}",
            "ROOM: ${WONTON_MATRIX_HOME_ROOM}\n      LEVEL: ${WONTON_SETTING_LEVEL}",
        )
        .replace(
            "  permissions: [matrix.bot]",
            "  settings: settings.schema.json\n  permissions: [matrix.bot]",
        )
}

#[tokio::test]
async fn writing_settings_recreates_the_bot_with_its_identity_kept() {
    let bed = Bed::start().await;
    let m = Arc::new(bed.manager());
    let files: PackageFiles = [(
        "settings.schema.json".to_owned(),
        LEVEL_SCHEMA.as_bytes().to_vec(),
    )]
    .into();
    let pkg = super::super::super::PluginPackage::with_files(
        &echo_with_setting(),
        files,
        &super::super::langs(),
    )
    .unwrap();
    m.install_user_enabled(
        pkg,
        &granted(),
        &[],
        false,
        &[],
        std::collections::BTreeMap::from([("level".to_owned(), json!("quiet"))]),
        super::super::langs(),
    )
    .await
    .unwrap();
    let before = bed.env();
    assert_eq!(before["LEVEL"], "quiet");

    let digest = m
        .plugin_settings(&id(), &super::super::langs())
        .await
        .unwrap()
        .schema_digest;
    let update = wonton_protocol::PluginSettingsUpdate {
        schema_digest: digest,
        values: std::collections::HashMap::from([(
            "level".to_owned(),
            Some(wonton_protocol::SettingValue::Text {
                value: "loud".to_owned(),
            }),
        )]),
    };
    let view = m
        .set_plugin_settings(&id(), &update, &super::super::langs())
        .await
        .unwrap();
    assert_eq!(view.form.fields[0].value, update.values["level"]);

    let after = bed.env();
    assert_eq!(after["LEVEL"], "loud");
    for key in ["HS", "USER", "TOKEN", "ALLOWED", "ROOM"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert!(bed.token_valid(&after["TOKEN"]));
}
