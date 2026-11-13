use super::*;
use serde_json::json;
use wonton_core::plugin::package_files::sha256_hex;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// 一个最小的插件包 compose：`extra` 是服务 `web` 里多出来的行。
fn compose(id: &str, extra: &[&str]) -> String {
    let extra: String = extra.iter().map(|line| format!("    {line}\n")).collect();
    format!(
        "services:\n  web:\n    image: nginx\n{extra}x-wonton:\n  spec: '0.4'\n  id: app.{id}\n  version: 1.0.0\n  kind: app\n  title: {{en: {id}}}\n  description: {{en: {id}}}\n  license: MIT\n  permissions: []\n"
    )
}

/// 索引里的一项：`files` 只有 compose，渲染用不到别的文件。
fn entry(source: &str, id: &str, extra: &[&str]) -> serde_json::Value {
    entry_with_compose(source, compose(id, extra))
}

fn entry_with_compose(source: &str, compose: String) -> serde_json::Value {
    serde_json::json!({
        "source": format!("github:acme/{source}"),
        "version": "1.0.0",
        "commit": COMMIT,
        "files": { COMPOSE_FILE: sha256_hex(compose.as_bytes()) },
        "compose": compose,
        "license": "MIT",
    })
}

fn capability_entry(id: &str, declarations: &str) -> serde_json::Value {
    let compose = compose(id, &[]).replace("spec: '0.4'", "spec: '0.11'");
    entry_with_compose(id, format!("{compose}{declarations}\n"))
}

/// 跑一遍 `run`：返回退出码对不对，和它打印出来的问题。
fn checked(plugins: Vec<serde_json::Value>) -> (bool, Vec<String>) {
    let json =
        serde_json::to_string_pretty(&serde_json::json!({ "format": 1, "plugins": plugins }))
            .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.json");
    std::fs::write(&path, &json).unwrap();
    let problems = problems(&Index::parse(json.as_bytes(), &Languages::default()).unwrap());
    let passed = run(&path) == ExitCode::SUCCESS;
    assert_eq!(passed, problems.is_empty(), "{problems:?}");
    (passed, problems)
}

#[test]
fn a_variable_the_core_does_not_inject_fails_the_index() {
    let (passed, problems) = checked(vec![
        entry(
            "broken",
            "broken",
            &["environment:\n      MATRIX_OWNER: ${WONTON_NOT_A_VAR}"],
        ),
        entry("fine", "fine", &[]),
    ]);
    assert!(!passed);
    assert_eq!(problems.len(), 1, "{problems:?}");
    let problem = &problems[0];
    assert!(problem.contains("github:acme/broken"), "{problem}");
    assert!(problem.contains("WONTON_NOT_A_VAR"), "{problem}");
    assert!(!problem.contains("github:acme/fine"), "{problem}");
}

#[test]
fn a_clean_index_passes() {
    let (passed, problems) = checked(vec![
        entry("first", "first", &["environment:\n      TZ: ${TZ}"]),
        entry("second", "second", &[]),
    ]);
    assert!(passed, "{problems:?}");
}

#[test]
fn unreadable_entries_fail_too() {
    let mut bad = entry("bad", "bad", &[]);
    bad["commit"] = "main".into();
    let (passed, problems) = checked(vec![bad]);
    assert!(!passed);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("github:acme/bad"), "{problems:?}");
}

/// 条目没带 license 时照常通过：线上索引还没有这些字段，缺不缺由生成时的
/// `plugin check --official` 管，这里只认写了的内容。
#[test]
fn an_entry_without_a_license_still_passes_the_index() {
    let mut bare = entry("bare", "bare", &[]);
    bare.as_object_mut().unwrap().remove("license");
    let compose = bare["compose"]
        .as_str()
        .unwrap()
        .replace("  license: MIT\n", "");
    bare["files"][COMPOSE_FILE] = json!(sha256_hex(compose.as_bytes()));
    bare["compose"] = json!(compose);
    let (passed, problems) = checked(vec![bare, entry("fine", "fine", &[])]);
    assert!(passed, "{problems:?}");
}

/// 索引里记的许可证和清单对不上：这一项读不懂，列出原因。
#[test]
fn a_license_that_does_not_match_the_manifest_fails_the_index() {
    let mut entry = entry("licensed", "licensed", &[]);
    entry["license"] = json!("AGPL-3.0-only");
    let (passed, problems) = checked(vec![entry]);
    assert!(!passed);
    assert!(problems[0].contains("license"), "{problems:?}");
    assert!(problems[0].contains("AGPL-3.0-only"), "{problems:?}");
}

/// 同一仓库里两个子目录的最后一段相同：tag 前缀会撞，`check-index` 挡下（PR #235 核对意见 ②）。
#[test]
fn two_subdirectories_with_the_same_last_segment_fail_the_index() {
    let (passed, problems) = checked(vec![
        entry("x/agents/codex", "codex", &[]),
        entry("x/apps/codex", "kimi", &[]),
        entry("x/apps/kimi", "other", &[]),
    ]);
    assert!(!passed);
    let [problem] = problems.as_slice() else {
        panic!("{problems:?}");
    };
    assert!(
        problem.contains("github:acme/x/agents/codex")
            && problem.contains("github:acme/x/apps/codex"),
        "{problem}"
    );
    assert!(!problem.contains("github:acme/x/apps/kimi"), "{problem}");
}

#[test]
fn required_capabilities_without_providers_fail_the_index() {
    for name in ["models", "torrent_client", "x-acme.ledger"] {
        let (passed, problems) = checked(vec![capability_entry(
            "consumer",
            &format!("  requires: [{name}]"),
        )]);
        assert!(!passed, "{name}");
        assert_eq!(problems.len(), 1, "{name}: {problems:?}");
    }
}

#[test]
fn optional_capabilities_without_providers_pass_the_index() {
    let (passed, problems) = checked(vec![capability_entry(
        "consumer",
        "  wants: [models, torrent_client, x-acme.ledger]",
    )]);
    assert!(passed, "{problems:?}");
}

#[test]
fn every_required_capability_needs_a_provider_but_one_plugin_can_provide_several() {
    let consumer = capability_entry("consumer", "  requires: [torrent_client, x-acme.ledger]");
    let torrent = "  provides:\n    torrent_client: {service: web, port: 8080}";
    let (passed, problems) = checked(vec![
        consumer.clone(),
        capability_entry("provider", torrent),
    ]);
    assert!(!passed);
    assert_eq!(problems.len(), 1, "{problems:?}");
    let (passed, problems) = checked(vec![
        consumer,
        capability_entry(
            "provider",
            &format!("{torrent}\n    x-acme.ledger: {{service: web, port: 8080}}"),
        ),
    ]);
    assert!(passed, "{problems:?}");
}
