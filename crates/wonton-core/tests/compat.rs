//! compat 的契约测试（ADR-0085 的缝一）：目标镜像 → 兼容裁决。两个提供方——真实子命令进程
//! （对本版本二进制跑 fixture 包）和 fake_host 模式的替身输出——喂同一个消费方
//! ([`interpret])，断言解析成同一份裁决；退出码和类别字符串按 ADR 逐字对照。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use wonton_core::plugin::compat::{self, Output};
use wonton_core::plugin::languages::Languages;
use wonton_core::plugin::store_info::SPEC_VERSION;
use wonton_core::upgrade::compat::interpret;
use wonton_core::upgrade::host::OneShotOutput;

fn langs() -> Languages {
    Languages::from_header(None)
}

/// 比这份核心新一版的规范版本：`spec` 的取值会随版本升，fixture 不钉死。
fn newer_spec() -> String {
    let (major, minor) = SPEC_VERSION
        .to_string()
        .split_once('.')
        .map(|(a, b)| (a.parse::<u32>().unwrap(), b.parse::<u32>().unwrap()))
        .unwrap();
    format!("{}.{}", major, minor + 1)
}

/// 一份 fixture 插件包目录（目录名用插件 ID）。`spec` 是清单的规范版本，`permissions`
/// 是权限行（含方括号），`extra` 是 `x-wonton` 下的其余额外行。
fn package(dir: &Path, id: &str, spec: &str, permissions: &str, extra: &str) -> PathBuf {
    let path = dir.join(id);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("docker-compose.yml"),
        format!(
            "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '{spec}'\n  id: {id}\n  \
             version: 1.0.0\n  kind: app\n  title: {{en: {id}}}\n  \
             description: {{en: {id}}}\n  permissions: {permissions}\n{extra}"
        ),
    )
    .unwrap();
    path
}

fn sent(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

fn run_subcommand(dirs: &[PathBuf]) -> OneShotOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_wonton-core"))
        .args(["plugin", "compat"])
        .args(dirs.iter().map(|dir| dir.display().to_string()))
        .output()
        .expect("跑真实子命令");
    OneShotOutput {
        exit_code: output.status.code().expect("退出码") as i64,
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// fake_host 的替身输出：和子命令同一份线上形状，内容手写。
fn fake_output() -> OneShotOutput {
    let output = serde_json::json!({
        "spec_supported": SPEC_VERSION.to_string(),
        "capabilities": ["models"],
        "plugins": [
            {"id": "app.ok.photo", "verdict": "ok"},
            {
                "id": "app.new.spec",
                "verdict": "breaking",
                "reasons": [{"category": "spec_unsupported", "raw": newer_spec()}],
            },
            {
                "id": "app.new.field",
                "verdict": "breaking",
                "reasons": [{"category": "unknown_feature", "raw": "x-wonton.qps"}],
            },
        ],
    });
    OneShotOutput {
        exit_code: 0,
        stdout: output.to_string(),
        stderr: String::new(),
    }
}

#[test]
fn the_real_subcommand_and_the_fake_output_parse_to_the_same_verdicts() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = vec![
        package(
            dir.path(),
            "app.ok.photo",
            &SPEC_VERSION.to_string(),
            "[]",
            "",
        ),
        package(dir.path(), "app.new.spec", &newer_spec(), "[]", ""),
        package(
            dir.path(),
            "app.new.field",
            &SPEC_VERSION.to_string(),
            "[]",
            "  qps: 5\n",
        ),
    ];
    let sent = sent(&["app.ok.photo", "app.new.spec", "app.new.field"]);

    let real = interpret(&run_subcommand(&dirs), "reg/core@x", &sent, &langs());
    let fake = interpret(&fake_output(), "reg/core@x", &sent, &langs());
    assert_eq!(
        real, fake,
        "两个提供方要解析成同一份裁决：{real:?} != {fake:?}"
    );

    let wonton_protocol::CoreUpgradeCompat::Breaking { plugins } = real else {
        panic!("{real:?}");
    };
    let reported: Vec<&str> = plugins.iter().map(|plugin| plugin.id.as_str()).collect();
    assert_eq!(reported, ["app.new.spec", "app.new.field"]);
    assert_eq!(plugins[0].reasons[0].raw, newer_spec());
    assert_eq!(plugins[1].reasons[0].raw, "x-wonton.qps");
}

/// 类别字符串逐字相同：子命令真实输出的每个类别都在四个固定字符串里，逐包对照。
#[test]
fn the_subcommand_emits_the_four_fixed_categories() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = vec![
        package(
            dir.path(),
            "app.ok.photo",
            &SPEC_VERSION.to_string(),
            "[]",
            "",
        ),
        package(dir.path(), "app.new.spec", &newer_spec(), "[]", ""),
        package(
            dir.path(),
            "app.new.field",
            &SPEC_VERSION.to_string(),
            "[]",
            "  qps: 5\n",
        ),
        package(
            dir.path(),
            "app.new.perm",
            &SPEC_VERSION.to_string(),
            "[files.home]",
            "",
        ),
        package(
            dir.path(),
            "app.new.cap",
            &SPEC_VERSION.to_string(),
            "[]",
            "  requires: [wonton_cloud]\n",
        ),
    ];

    let output = run_subcommand(&dirs);
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    let parsed: Output = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(parsed.spec_supported, SPEC_VERSION.to_string());
    assert!(parsed.capabilities.iter().any(|name| name == "models"));
    let allowed = [
        compat::SPEC_UNSUPPORTED,
        compat::UNKNOWN_FEATURE,
        compat::CAPABILITY_MISSING,
        compat::OTHER,
    ];
    for verdict in &parsed.plugins {
        for reason in &verdict.reasons {
            assert!(
                allowed.contains(&reason.category.as_str()),
                "不认识的类别 {}（raw {}）",
                reason.category,
                reason.raw
            );
        }
    }
    let by_id: BTreeMap<String, Vec<String>> = parsed
        .plugins
        .into_iter()
        .map(|verdict| {
            (
                verdict.id,
                verdict.reasons.iter().map(|r| r.category.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(by_id["app.new.spec"], ["spec_unsupported"]);
    assert_eq!(by_id["app.new.field"], ["unknown_feature"]);
    assert_eq!(by_id["app.new.perm"], ["unknown_feature"]);
    assert_eq!(by_id["app.new.cap"], ["capability_missing"]);
    assert!(by_id["app.ok.photo"].is_empty());
}

/// 退出码语义：裁决完成是 0（有 breaking 也是 0）；用法不对是 2，消费方按 Unavailable 处理。
#[test]
fn exit_codes_keep_their_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = vec![package(dir.path(), "app.new.spec", &newer_spec(), "[]", "")];
    // 有失效的插件也是 0。
    let output = run_subcommand(&dirs);
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    let sent = sent(&["app.new.spec"]);
    let parsed: Output = serde_json::from_str(&output.stdout).unwrap();
    assert!(
        parsed
            .plugins
            .iter()
            .any(|verdict| verdict.verdict == compat::Verdict::Breaking),
        "{}",
        output.stdout
    );

    // 用法不对（没给目录）是 2：目标镜像太老没有这个子命令时，升级方看到的也是它。
    let usage = run_subcommand(&[]);
    assert_eq!(usage.exit_code, 2, "{}", usage.stderr);
    assert_eq!(
        interpret(&usage, "reg/core@x", &sent, &langs()),
        wonton_protocol::CoreUpgradeCompat::Unavailable
    );
}
