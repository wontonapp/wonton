use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::models::ModelsRoute;
use super::super::names::container_name;
use super::super::storefront::StoreConfig;
use super::super::{PluginManager, PluginPackage, PluginStatus};
use super::manager::manager_with;
use std::fs;
use std::sync::{Arc, Mutex};

const PROVIDER: &str = "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.6'\n  id: system.models\n  version: 0.6.0\n  kind: system\n  title: {zh: 模型, en: Models}\n  description: {zh: 模型, en: Models}\n  webui: {service: magpie, port: 3430}\n  provides:\n    models:\n      service: magpie\n      port: 3425\n      apis: {openai: /v1}\n      default_model: group/default\n      credentials:\n        issue: [issue, '{name}']\n        revoke: [revoke, '{name}']\n      providers:\n        list: [providers, list]\n        save: [providers, save, '{id}']\n  permissions: []";

fn command(cmd: &[String], expected: &[&str]) -> bool {
    cmd.iter().map(String::as_str).eq(expected.iter().copied())
}

fn manager_with_index(
    engine: super::super::docker::Engine,
    dir: &tempfile::TempDir,
) -> PluginManager {
    manager_with(engine, dir).with_store(StoreConfig {
        index_dir: Some(dir.path().join("official-index")),
        community: false,
    })
}

fn write_recommendations(dir: &tempfile::TempDir) {
    let index = dir.path().join("official-index");
    fs::create_dir_all(&index).unwrap();
    fs::write(
        index.join("index.json"),
        r#"{"format":1,"plugins":[],"onboarding":{"agents":[],"needs":[],"providers":{"en":[{"id":"deepseek","key_url":"https://platform.deepseek.com/api_keys"}]},"prompts":[]}}"#,
    )
    .unwrap();
}

struct Bed {
    docker: FakeDocker,
    manager: PluginManager,
    _dir: tempfile::TempDir,
}

async fn bed() -> Bed {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_container(&container_name("system-models", "magpie"), "system.models");
    let output = Arc::new(Mutex::new((
        0_i64,
        r#"{"presets":[{"id":"deepseek","title":"DeepSeek"},{"id":"openrouter","title":"OpenRouter"}],"configured":["openrouter"]}"#.to_owned(),
        String::new(),
    )));
    let command_output = output.clone();
    docker.on_exec_output(move |_, cmd, stdin| {
        if command(cmd, &["providers", "list"]) {
            let result = command_output.lock().unwrap().clone();
            return (result.0, result.1, result.2);
        }
        if command(cmd, &["providers", "save", "deepseek"]) {
            assert_eq!(stdin, b"secret-key\n");
            return (0, String::new(), String::new());
        }
        (2, String::new(), "unknown command".to_owned())
    });
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_index(engine, &dir);
    register_gateway(&manager);
    Bed {
        docker,
        manager,
        _dir: dir,
    }
}

fn register_gateway(manager: &PluginManager) {
    let manifest = PluginPackage::parse(PROVIDER).unwrap().manifest;
    let route = ModelsRoute {
        id: manifest.id.clone(),
        spec: manifest.provides.clone().unwrap().models.unwrap(),
        upstream: "127.0.0.1:3425".into(),
        credentials_container: container_name("system-models", "magpie"),
    };
    manager.register_for_test(manifest, PluginStatus::Running, None);
    manager.serve_models_for_test(route);
}

#[tokio::test]
async fn lists_presets_and_merges_recommendations() {
    let bed = bed().await;
    write_recommendations(&bed._dir);
    let result = bed
        .manager
        .model_providers(&Languages::from_header(Some("en")))
        .await
        .unwrap();
    let providers = result.providers.unwrap();
    assert_eq!(providers[0].id, "deepseek");
    assert!(providers[0].recommended);
    assert_eq!(
        providers[0].key_url.as_deref(),
        Some("https://platform.deepseek.com/api_keys")
    );
    assert_eq!(providers[1].id, "openrouter");
    assert!(providers[1].configured);
    assert!(!providers[1].recommended);
}

#[tokio::test]
async fn index_failure_does_not_hide_gateway_presets() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_container(&container_name("system-models", "magpie"), "system.models");
    docker.on_exec_output(|_, _, _| {
        (
            0,
            r#"{"presets":[{"id":"deepseek","title":"DeepSeek"}],"configured":[]}"#.into(),
            String::new(),
        )
    });
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_index(engine, &dir);
    register_gateway(&manager);
    let result = manager
        .model_providers(&Languages::from_header(Some("en")))
        .await
        .unwrap();
    assert_eq!(result.providers.unwrap()[0].id, "deepseek");
}

#[tokio::test]
async fn save_passes_key_only_on_stdin() {
    let bed = bed().await;
    bed.manager
        .save_model_provider("deepseek", "secret-key")
        .await
        .unwrap();
    let exec = bed.docker.execs().into_iter().last().unwrap();
    assert_eq!(exec.stdin, b"secret-key\n");
    assert!(!exec.cmd.iter().any(|arg| arg.contains("secret-key")));
}

#[tokio::test]
async fn save_rejects_unknown_provider_without_running_save() {
    let bed = bed().await;
    let error = bed
        .manager
        .save_model_provider("unknown", "secret-key")
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(bed.docker.execs().len(), 1, "只执行 list");
}

#[tokio::test]
async fn malformed_list_is_models_unavailable() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_container(&container_name("system-models", "magpie"), "system.models");
    docker.on_exec_output(|_, _, _| (0, "not json".into(), String::new()));
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_index(engine, &dir);
    register_gateway(&manager);
    let error = manager
        .model_providers(&Languages::default())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "models_unavailable");
}

#[tokio::test]
async fn save_exit_codes_have_distinct_error_kinds_without_stderr_leak() {
    for (code, kind) in [
        (3, "invalid_input"),
        (4, "upstream_unavailable"),
        (9, "models_unavailable"),
    ] {
        let (docker, engine) = FakeDocker::start().await;
        docker.add_container(&container_name("system-models", "magpie"), "system.models");
        docker.on_exec_output(move |_, cmd, _| {
            if command(cmd, &["providers", "list"]) {
                return (
                    0,
                    r#"{"presets":[{"id":"deepseek","title":"DeepSeek"}],"configured":[]}"#.into(),
                    String::new(),
                );
            }
            (code, String::new(), "secret-key must not escape".into())
        });
        let dir = tempfile::tempdir().unwrap();
        let manager = manager_with_index(engine, &dir);
        register_gateway(&manager);
        let error = manager
            .save_model_provider("deepseek", "secret-key")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert!(!error.to_string().contains("secret-key"));
    }
}

#[test]
fn provider_manifest_requires_nonempty_commands_and_id_placeholder() {
    for (list, save, expected) in [
        ("[]", "[providers, save, '{id}']", "list"),
        ("[providers, list]", "[]", "save"),
        ("[providers, list]", "[providers, save]", "{id}"),
    ] {
        let yaml = PROVIDER
            .replace("list: [providers, list]", &format!("list: {list}"))
            .replace("save: [providers, save, '{id}']", &format!("save: {save}"));
        let error = PluginPackage::parse(&yaml).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[tokio::test]
async fn invalid_keys_are_rejected_before_save() {
    for key in ["", "a\nkey"] {
        let bed = bed().await;
        let error = bed
            .manager
            .save_model_provider("deepseek", key)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
    }
    let bed = bed().await;
    let key = "a".repeat(4097);
    let error = bed
        .manager
        .save_model_provider("deepseek", &key)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
}
