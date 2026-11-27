use std::fs::File;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use wonton_onboard::cleanup::sweep_roots;
use wonton_onboard::process::Owner;

fn write(path: &std::path::Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

#[test]
fn sweeps_app_shared_and_temp_roots() {
    let dir = TempDir::new().unwrap();
    let roots: Vec<_> = (0..3)
        .map(|index| {
            let root = dir.path().join(format!("root-{index}"));
            std::fs::create_dir_all(root.join(".git")).unwrap();
            root
        })
        .collect();

    for root in &roots {
        write(&root.join(".state.toml.lock"), "pid:4000000:1");
        write(&root.join(".git/wonton-finish.json"), r#"{"prs":{}}"#);
        let temp = root.join(".state.toml.1.tmp");
        File::create(&temp)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(7_200))
            .unwrap();
        write(&root.join(".state.toml.2.tmp"), "fresh");
    }

    let removed = sweep_roots(&roots).unwrap();
    assert_eq!(removed.len(), 9, "{removed:?}");
    for root in &roots {
        assert!(!root.join(".state.toml.lock").exists());
        assert!(!root.join(".git/wonton-finish.json").exists());
        assert!(!root.join(".state.toml.1.tmp").exists());
        assert!(root.join(".state.toml.2.tmp").exists());
    }
}

#[test]
fn keeps_live_lock_and_claimed_notes() {
    let dir = TempDir::new().unwrap();
    let owner = Owner::current().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    write(&root.join(".state.toml.lock"), &owner.to_string());
    write(
        &root.join(".state.toml"),
        r#"
[records.cycle-1]
status = "in_progress"

[batches.cycle-1]
"#,
    );
    write(&root.join(".git/wonton-finish.json"), r#"{"prs":{}}"#);

    let removed = sweep_roots(std::slice::from_ref(&root)).unwrap();
    assert!(removed.is_empty(), "{removed:?}");
    assert!(root.join(".state.toml.lock").exists());
    assert!(root.join(".git/wonton-finish.json").exists());
}
