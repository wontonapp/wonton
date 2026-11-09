//! 主人的设置不对时，真的 `wonton-core` 报错退出（ADR-0022）。这些情况都在连 Docker 之前就失败，所以不要 Docker。

use std::path::Path;
use std::process::{Command, Output};

use wonton_core::owner::lock_username;

const WONTON_CORE: &str = env!("CARGO_BIN_EXE_wonton-core");

fn run(args: &[&str], vars: &[(&str, &str)]) -> Output {
    Command::new(WONTON_CORE)
        .args(args)
        .env_clear()
        .env("RUST_LOG", "error")
        .envs(vars.iter().copied())
        .output()
        .unwrap()
}

fn failure(output: &Output) -> String {
    assert!(!output.status.success(), "应该报错退出");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn data_dir(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

#[test]
fn the_core_refuses_to_start_when_the_owner_name_changed() {
    let dir = tempfile::tempdir().unwrap();
    lock_username(&dir.path().join("core"), Some("alice")).unwrap();
    let vars = [
        ("WONTON_DATA_DIR", data_dir(dir.path())),
        ("WONTON_OWNER_NAME", "bob"),
    ];
    let log = failure(&run(&["serve"], &vars));
    for needed in ["bob", "alice", "owner.json"] {
        assert!(log.contains(needed), "{log}");
    }
}

#[test]
fn the_core_refuses_to_start_with_bad_owner_settings() {
    let dir = tempfile::tempdir().unwrap();
    for (name, value) in [
        ("WONTON_OWNER_NAME", "a/b"),
        ("WONTON_OWNER_NAME", "nobody"),
        ("TZ", "CST-8"),
    ] {
        let vars = [("WONTON_DATA_DIR", data_dir(dir.path())), (name, value)];
        let log = failure(&run(&["serve"], &vars));
        assert!(log.contains(name), "{value}：{log}");
    }
    assert!(
        !dir.path().join("core/owner.json").exists(),
        "不合规则的值没被记下"
    );
}

#[test]
fn init_reports_bad_owner_settings_before_writing_anything() {
    let data = tempfile::tempdir().unwrap();
    lock_username(&data.path().join("core"), Some("alice")).unwrap();
    let install = tempfile::tempdir().unwrap();
    let init = ["init", install.path().to_str().unwrap(), "wonton.localhost"];
    let log = failure(&run(&init, &[("TZ", "CST-8")]));
    assert!(log.contains("TZ"), "{log}");
    let changed = [
        ("WONTON_DATA_DIR", data_dir(data.path())),
        ("WONTON_OWNER_NAME", "bob"),
    ];
    let log = failure(&run(&init, &changed));
    assert!(log.contains("alice"), "{log}");
    assert!(!install.path().join("secrets").exists());

    let fresh = tempfile::tempdir().unwrap();
    let reserved = [
        ("WONTON_DATA_DIR", data_dir(fresh.path())),
        ("WONTON_OWNER_NAME", "conduit"),
    ];
    let log = failure(&run(&init, &reserved));
    assert!(log.contains("conduit"), "{log}");
    assert!(!fresh.path().join("core/owner.json").exists());
}

#[test]
fn init_needs_no_owner_settings() {
    // 一键升级在新镜像里执行 `init`，什么都不传。
    let install = tempfile::tempdir().unwrap();
    let init = ["init", install.path().to_str().unwrap(), "wonton.localhost"];
    let output = run(&init, &[]);
    assert!(output.status.success(), "{output:?}");
}
