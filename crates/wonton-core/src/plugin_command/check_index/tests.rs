use super::*;
use wonton_core::plugin::package_files::sha256_hex;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// 一个最小的插件包 compose：`extra` 是服务 `web` 里多出来的行。
fn compose(id: &str, extra: &[&str]) -> String {
    let extra: String = extra.iter().map(|line| format!("    {line}\n")).collect();
    format!(
        "services:\n  web:\n    image: nginx\n{extra}x-wonton:\n  spec: '0.4'\n  id: app.{id}\n  version: 1.0.0\n  kind: app\n  title: {{en: {id}}}\n  description: {{en: {id}}}\n  permissions: []\n"
    )
}

/// 索引里的一项：`files` 只有 compose，渲染用不到别的文件。
fn entry(source: &str, id: &str, extra: &[&str]) -> serde_json::Value {
    let compose = compose(id, extra);
    serde_json::json!({
        "source": format!("github:acme/{source}"),
        "version": "1.0.0",
        "commit": COMMIT,
        "files": { COMPOSE_FILE: sha256_hex(compose.as_bytes()) },
        "compose": compose,
    })
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
