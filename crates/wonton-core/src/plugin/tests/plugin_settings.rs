//! 插件设置（ADR-0077）：Schema 子集在预览时拦下，值存进记录、注入变量，写设置只重建用到的服务。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use wonton_protocol::{InstallRequest, SettingValue, SettingValues, SettingsFieldProblemKind};

use super::super::fake_docker::FakeDocker;
use super::super::install_error::InstallError;
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::super::package::PluginPackage;
use super::super::package_files::PackageFiles;
use super::manager::manager_with;

fn langs() -> Languages {
    Languages::default()
}

const SCHEMA_PATH: &str = "settings.schema.json";

const SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "x-wonton-order": ["budget", "months"],
  "properties": {
    "months": {
      "type": "integer",
      "minimum": 1,
      "maximum": 12,
      "default": 3,
      "x-wonton-title": { "zh": "显示几个月", "en": "Months shown" }
    },
    "budget": {
      "type": "string",
      "enum": ["household", "personal"],
      "default": "household",
      "x-wonton-title": { "zh": "预算", "en": "Budget" },
      "x-wonton-enum-titles": {
        "household": { "zh": "家庭", "en": "Household" },
        "personal": { "zh": "个人", "en": "Personal" }
      }
    }
  }
}"#;

/// 两个服务：`web` 引用设置，`worker` 不引用。`schema` 换成别的写法测子集校验。
fn compose(schema: &str) -> String {
    let services = "  web:\n    image: nginx\n    environment:\n      BUDGET: '${WONTON_SETTING_BUDGET}'\n  \
                    worker:\n    image: nginx";
    super::package(
        services,
        "  permissions: []\n  settings: settings.schema.json",
    )
    .replace("<schema>", schema)
}

fn bundle(compose: &str, schema: &str) -> PluginPackage {
    let files: PackageFiles = [(SCHEMA_PATH.to_owned(), schema.as_bytes().to_vec())].into();
    PluginPackage::with_files(compose, files, &langs()).unwrap()
}

/// 预览上传的包，返回安装请求要的来源和摘要。
async fn staged(m: &super::super::manager::PluginManager, pkg: PluginPackage) -> (String, String) {
    let preview = m.preview_upload(pkg, &langs()).await.unwrap();
    (preview.source.clone().unwrap(), preview.digest.clone())
}

fn request(source: &str, digest: &str, settings: SettingValues) -> InstallRequest {
    InstallRequest {
        scope: None,
        audience: None,
        source: source.to_owned(),
        digest: digest.to_owned(),
        plan: String::new(),
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings,
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    }
}

fn setting(text: &str) -> SettingValue {
    SettingValue::Text {
        value: text.to_owned(),
    }
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

#[tokio::test]
async fn preview_rejects_schemas_outside_the_native_subset() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let nested = SCHEMA.replace(
        r#""type": "string","#,
        r#""type": "object","properties": {"inner": {"type": "string"}},"#,
    );
    let pkg = bundle(&compose("<schema>"), &nested);
    let err = m.preview_install(&pkg, &langs()).await.unwrap_err();
    let InstallError::SettingsSchemaInvalid(_, problems) = &err else {
        panic!("{err}");
    };
    assert!(
        problems
            .iter()
            .any(|p| p.kind == wonton_protocol::SettingsSchemaProblemKind::UnsupportedKeyword),
        "{problems:?}"
    );
}

#[tokio::test]
async fn preview_shows_defaults_titles_and_order() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let pkg = bundle(&compose(SCHEMA), SCHEMA);
    let preview = m.preview_install(&pkg, &langs()).await.unwrap();
    let form = preview.settings.expect("有设置表单");
    assert_eq!(
        form.fields
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["budget", "months"]
    );
    assert_eq!(form.fields[1].title, "Months shown");
    assert_eq!(
        form.fields[1].default,
        Some(SettingValue::Integer { value: 3 })
    );
    let options = match &form.fields[0].kind {
        wonton_protocol::PluginSettingKind::Choice { options, multiple } => {
            assert!(!multiple);
            options.clone()
        }
        kind => panic!("{kind:?}"),
    };
    assert_eq!(options[0].title, "Household");
}

#[tokio::test]
async fn install_stores_values_and_rejects_bad_ones() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let pkg = bundle(&compose(SCHEMA), SCHEMA);
    let (source, digest) = staged(&m, pkg.clone()).await;

    let mut settings = SettingValues::new();
    settings.insert("months".to_owned(), SettingValue::Integer { value: 40 });
    let err = m
        .install_source(&request(&source, &digest, settings))
        .await
        .unwrap_err();
    let InstallError::SettingsInvalid(_, fields) = &err else {
        panic!("{err}");
    };
    assert_eq!(fields[0].name, "months");
    assert_eq!(fields[0].kind, SettingsFieldProblemKind::AboveMaximum);

    // 再预览一次拿同一份包（第一次失败没有消费暂存），这次值合法。
    let mut settings = SettingValues::new();
    settings.insert("budget".to_owned(), setting("personal"));
    m.install_source(&request(&source, &digest, settings))
        .await
        .unwrap();
    let record = m.store.load(&id()).unwrap().unwrap();
    assert_eq!(record.settings["budget"], json!("personal"));
    // 没填的 months 用默认值。
    assert_eq!(record.settings["months"], json!(3));
    assert!(m.judged_detail(&id(), &langs()).await.unwrap().has_settings);
}

#[tokio::test]
async fn writing_settings_rebuilds_only_the_service_using_it() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let pkg = bundle(&compose(SCHEMA), SCHEMA);
    let (source, digest) = staged(&m, pkg).await;
    m.install_source(&request(&source, &digest, Default::default()))
        .await
        .unwrap();
    let before = docker.writes();

    let update = wonton_protocol::PluginSettingsUpdate {
        schema_digest: m
            .plugin_settings(&id(), &langs())
            .await
            .unwrap()
            .schema_digest,
        values: HashMap::from([("budget".to_owned(), Some(setting("personal")))]),
    };
    let view = m
        .set_plugin_settings(&id(), &update, &langs())
        .await
        .unwrap();
    assert_eq!(view.form.fields[0].value, Some(setting("personal")));

    let writes = docker.writes()[before.len()..].to_vec();
    assert!(
        writes.iter().any(|w| w.contains("web")),
        "重建了 web：{writes:?}"
    );
    assert!(
        !writes.iter().any(|w| w.contains("worker")),
        "worker 没被重建：{writes:?}"
    );
    let record = m.store.load(&id()).unwrap().unwrap();
    assert_eq!(record.settings["budget"], json!("personal"));
    assert!(record.settings_changed_ms.is_some());
}

#[tokio::test]
async fn a_changed_schema_file_refuses_the_write() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let pkg = bundle(&compose(SCHEMA), SCHEMA);
    let (source, digest) = staged(&m, pkg).await;
    m.install_source(&request(&source, &digest, Default::default()))
        .await
        .unwrap();
    let update = wonton_protocol::PluginSettingsUpdate {
        schema_digest: "sha256:stale".to_owned(),
        values: Default::default(),
    };
    let err = m
        .set_plugin_settings(&id(), &update, &langs())
        .await
        .unwrap_err();
    assert!(matches!(err, InstallError::Changed(_)), "{err}");
}

/// 同值写入不重建任何容器。
#[tokio::test]
async fn the_same_value_does_not_rebuild() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let pkg = bundle(&compose(SCHEMA), SCHEMA);
    let (source, digest) = staged(&m, pkg).await;
    m.install_source(&request(&source, &digest, Default::default()))
        .await
        .unwrap();
    let before = docker.writes();

    let update = wonton_protocol::PluginSettingsUpdate {
        schema_digest: m
            .plugin_settings(&id(), &langs())
            .await
            .unwrap()
            .schema_digest,
        values: HashMap::from([("budget".to_owned(), Some(setting("household")))]),
    };
    m.set_plugin_settings(&id(), &update, &langs())
        .await
        .unwrap();
    assert_eq!(docker.writes().len(), before.len());
}
