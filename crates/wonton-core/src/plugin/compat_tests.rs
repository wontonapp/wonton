use super::*;
use crate::plugin::package_files::PackageFiles;

const FIELDS: &str = "  spec: '0.14'\n  id: app.example.photos\n  version: 1.0.0\n  kind: app\n  \
                      title: {en: Example}\n  description: {en: Example}\n  permissions: []\n";

/// `extra` 直接放在 `x-wonton:` 下（两格缩进起），`service` 放在服务 `web` 下（四格缩进起）。
/// `fields` 覆盖清单字段（含 permissions），默认 FIELDS。
fn compose_with(fields: &str, extra: &str, service: &str) -> String {
    format!("services:\n  web:\n    image: nginx\n{service}x-wonton:\n{fields}{extra}")
}

fn compose(extra: &str, service: &str) -> String {
    compose_with(FIELDS, extra, service)
}

fn judged(source: &str) -> PackageVerdict {
    judge(source, &Languages::default(), "dir.example", |langs| {
        PluginPackage::with_files(source, PackageFiles::new(), langs)
    })
}

#[test]
fn a_readable_package_is_ok() {
    let verdict = judged(&compose("", ""));
    assert_eq!(verdict.id, "app.example.photos");
    assert_eq!(verdict.verdict, Verdict::Ok);
    assert!(verdict.reasons.is_empty(), "{:?}", verdict.reasons);
}

#[test]
fn a_newer_spec_is_reported_before_any_field() {
    let source = compose("  not_a_field: 1\n", "").replacen("spec: '0.14'", "spec: '0.15'", 1);
    let verdict = judged(&source);
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(SPEC_UNSUPPORTED, "0.15")],
        "{:?}",
        verdict.reasons
    );
}

#[test]
fn an_unknown_field_is_reported_with_its_path() {
    let verdict = judged(&compose("  qps: 5\n", ""));
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(UNKNOWN_FEATURE, "x-wonton.qps")],
        "{:?}",
        verdict.reasons
    );
}

#[test]
fn an_unknown_permission_is_reported_by_name() {
    let fields = FIELDS.replace(
        "  permissions: []\n",
        "  permissions: [files.home, models]\n",
    );
    let verdict = judged(&compose_with(&fields, "", ""));
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(UNKNOWN_FEATURE, "files.home")],
        "{:?}",
        verdict.reasons
    );
}

#[test]
fn a_missing_capability_is_reported_by_name() {
    let verdict = judged(&compose("  requires: [models, wonton_cloud]\n", ""));
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(CAPABILITY_MISSING, "wonton_cloud")],
        "{:?}",
        verdict.reasons
    );
}

/// 安装路径只报第一个不认识的名字；compat 把权限和能力一次报全。
#[test]
fn all_unknown_names_are_reported_at_once() {
    let fields = FIELDS.replace(
        "  permissions: []\n",
        "  permissions: [files.home]\n  requires: [wonton_cloud]\n",
    );
    let verdict = judged(&compose_with(&fields, "", ""));
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![
            Reason::new(UNKNOWN_FEATURE, "files.home"),
            Reason::new(CAPABILITY_MISSING, "wonton_cloud"),
        ],
        "{:?}",
        verdict.reasons
    );
}

#[test]
fn a_variable_the_core_does_not_inject_fails_rendering() {
    let verdict = judged(&compose(
        "",
        "    environment:\n      TZ: ${WONTON_NOT_INJECTED}\n",
    ));
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(OTHER, "WONTON_NOT_INJECTED")],
        "{:?}",
        verdict.reasons
    );
}

#[test]
fn an_unreadable_package_breaks_without_blocking_others() {
    // YAML 坏了：没有 id 可读，回落到调用方给的目录名。
    let verdict = judged("services: [broken");
    assert_eq!(verdict.id, "dir.example");
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(OTHER, "")],
        "{:?}",
        verdict.reasons
    );
}

#[test]
fn a_manifest_without_spec_is_other() {
    let source = compose("", "").replacen("  spec: '0.14'\n", "", 1);
    let verdict = judged(&source);
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(
        verdict.reasons,
        vec![Reason::new(OTHER, "")],
        "{:?}",
        verdict.reasons
    );
}

/// 目录裁决走 [`judge_dir`]：读 compose 和被引用的文件，坏包用目录名当 id。
#[test]
fn a_directory_is_judged_from_its_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(COMPOSE_FILE), compose("", "")).unwrap();
    let verdict = judge_dir(dir.path(), &Languages::default());
    assert_eq!(verdict.verdict, Verdict::Ok);

    let empty = tempfile::tempdir().unwrap();
    let verdict = judge_dir(empty.path(), &Languages::default());
    assert_eq!(
        verdict.id,
        empty.path().file_name().unwrap().to_string_lossy()
    );
    assert_eq!(verdict.verdict, Verdict::Breaking);
    assert_eq!(verdict.reasons, vec![Reason::new(OTHER, "")]);
}
