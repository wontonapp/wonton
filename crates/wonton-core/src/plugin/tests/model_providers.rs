use super::super::fake_docker::FakeDocker;
use super::super::model_providers::ProviderAction;
use super::super::models::ModelsRoute;
use super::super::names::container_name;
use super::super::storefront::StoreConfig;
use super::super::{PluginManager, PluginPackage, PluginStatus};
use super::manager::manager_with;
use std::sync::{Arc, Mutex};
use wonton_protocol::{CustomModelProvider, SecretString};

const PROVIDER: &str = "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.6'\n  id: system.models\n  version: 0.6.0\n  kind: system\n  title: {zh: 模型, en: Models}\n  description: {zh: 模型, en: Models}\n  webui: {service: magpie, port: 3430}\n  provides:\n    models:\n      service: magpie\n      port: 3425\n      apis: {openai: /v1}\n      default_model: group/default\n      credentials:\n        issue: [issue, '{name}']\n        revoke: [revoke, '{name}']\n      providers:\n        list: [providers, list]\n        save: [providers, save, '{id}']\n        test: [providers, test, '{id}']\n        custom:\n          save: [providers, custom-save]\n          test: [providers, custom-test]\n  permissions: []";

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
async fn save_passes_key_only_on_stdin() {
    let bed = bed().await;
    bed.manager
        .model_provider_key("deepseek", "secret-key", ProviderAction::Save)
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
        .model_provider_key("unknown", "secret-key", ProviderAction::Save)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(bed.docker.execs().len(), 1, "只执行 list");
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
            .model_provider_key("deepseek", "secret-key", ProviderAction::Save)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert!(!error.to_string().contains("secret-key"));
    }
}

#[test]
fn provider_manifest_requires_commands_and_valid_placeholders() {
    for (original, replacement) in [
        ("list: [providers, list]", "list: []"),
        ("save: [providers, save, '{id}']", "save: []"),
        ("save: [providers, save, '{id}']", "save: [providers, save]"),
        ("test: [providers, test, '{id}']", "test: []"),
        ("test: [providers, test, '{id}']", "test: [providers, test]"),
        ("        test: [providers, test, '{id}']\n", ""),
        ("save: [providers, custom-save]", "save: []"),
        ("test: [providers, custom-test]", "test: []"),
        ("save: [providers, custom-save]", "save: [custom, '{id}']"),
        ("test: [providers, custom-test]", "test: [custom, '{key}']"),
    ] {
        assert!(PluginPackage::parse(&PROVIDER.replace(original, replacement)).is_err());
    }
}

#[tokio::test]
async fn invalid_keys_are_rejected_before_save() {
    for key in ["", "a\nkey"] {
        let bed = bed().await;
        let error = bed
            .manager
            .model_provider_key("deepseek", key, ProviderAction::Save)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(bed.docker.execs().is_empty());
    }
    let bed = bed().await;
    let key = "a".repeat(4097);
    let error = bed
        .manager
        .model_provider_key("deepseek", &key, ProviderAction::Save)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(bed.docker.execs().is_empty());
}

fn custom_provider() -> CustomModelProvider {
    CustomModelProvider {
        base_url: "http://192.0.2.10:8000/v1".into(),
        model: "vendor/model".into(),
        key: SecretString::new("secret-\"key\\"),
    }
}

#[tokio::test]
async fn test_passes_key_only_on_stdin_and_rejects_unknown_ids() {
    let bed = bed().await;
    bed.docker.on_exec_output(|_, cmd, _| {
        if command(cmd, &["providers", "list"]) {
            return (
                0,
                r#"{"presets":[{"id":"deepseek","title":"DeepSeek"}],"configured":[]}"#.into(),
                String::new(),
            );
        }
        (0, String::new(), String::new())
    });
    bed.manager
        .model_provider_key("deepseek", "secret-key", ProviderAction::Test)
        .await
        .unwrap();
    let exec = bed.docker.execs().into_iter().last().unwrap();
    assert_eq!(exec.cmd, ["providers", "test", "deepseek"]);
    assert_eq!(exec.stdin, b"secret-key\n");
    let error = bed
        .manager
        .model_provider_key("unknown", "secret-key", ProviderAction::Test)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(
        bed.docker
            .execs()
            .iter()
            .all(|exec| !exec.cmd.iter().any(|arg| arg == "unknown"))
    );
}

#[tokio::test]
async fn custom_commands_receive_one_json_line_only_on_stdin() {
    for (action, expected) in [
        (ProviderAction::Save, "custom-save"),
        (ProviderAction::Test, "custom-test"),
    ] {
        let bed = bed().await;
        bed.docker
            .on_exec_output(|_, _, _| (0, String::new(), String::new()));
        let provider = custom_provider();
        bed.manager
            .custom_model_provider(&provider, action)
            .await
            .unwrap();
        let execs = bed.docker.execs();
        assert_eq!(execs.len(), 1);
        assert_eq!(execs[0].cmd, ["providers", expected]);
        let mut expected = serde_json::to_vec(&provider).unwrap();
        expected.push(b'\n');
        assert_eq!(execs[0].stdin, expected);
        assert_eq!(
            execs[0].stdin.iter().filter(|&&byte| byte == b'\n').count(),
            1
        );
    }
}

#[tokio::test]
async fn invalid_custom_input_never_executes_any_command() {
    let long_url = format!("https://example.com/{}", "a".repeat(2048));
    for url in [
        "",
        "not-a-url",
        "file:///etc/passwd",
        "https://",
        "https://example.com/a b",
        "http://example.com/\t",
        "https://example.com/\u{2003}",
        &long_url,
    ] {
        let bed = bed().await;
        let mut provider = custom_provider();
        provider.base_url = url.into();
        assert_eq!(
            bed.manager
                .custom_model_provider(&provider, ProviderAction::Save)
                .await
                .unwrap_err()
                .kind(),
            "invalid_input"
        );
        assert!(bed.docker.execs().is_empty());
    }
    for model in [
        "",
        "a b",
        "x\t",
        "x\0",
        "x\u{7f}",
        "x\u{2003}",
        &"a".repeat(201),
        &"模".repeat(67),
    ] {
        let bed = bed().await;
        let mut provider = custom_provider();
        provider.model = model.into();
        assert_eq!(
            bed.manager
                .custom_model_provider(&provider, ProviderAction::Test)
                .await
                .unwrap_err()
                .kind(),
            "invalid_input"
        );
        assert!(bed.docker.execs().is_empty());
    }
    for key in ["", "x\r", "x\n", &"a".repeat(4097)] {
        let bed = bed().await;
        let mut provider = custom_provider();
        provider.key = SecretString::new(key);
        assert_eq!(
            bed.manager
                .custom_model_provider(&provider, ProviderAction::Save)
                .await
                .unwrap_err()
                .kind(),
            "invalid_input"
        );
        assert!(bed.docker.execs().is_empty());
    }
}

#[tokio::test]
async fn custom_accepts_boundary_lengths_without_normalizing_input() {
    let bed = bed().await;
    bed.docker
        .on_exec_output(|_, _, _| (0, String::new(), String::new()));
    let mut provider = custom_provider();
    let base = "http://example.com/";
    provider.base_url = format!("{base}{}", "a".repeat(2048 - base.len()));
    provider.model = "m".repeat(200);
    provider.key = SecretString::new("k".repeat(4096));
    bed.manager
        .custom_model_provider(&provider, ProviderAction::Test)
        .await
        .unwrap();
    let exec = bed.docker.execs().pop().unwrap();
    let sent: CustomModelProvider = serde_json::from_slice(&exec.stdin).unwrap();
    assert_eq!(sent, provider);
}
