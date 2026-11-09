//! 每个人的语言怎么落盘（ADR-0043）：读不出来时整份表会被一个人的记录盖掉，所以要停手；没有可用的语言时不写。

use std::fs;
use std::path::PathBuf;

use super::manager::manager_with;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::store::read_people_languages;

fn file(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("plugins/store/people-languages.json")
}

fn write_file(dir: &tempfile::TempDir, text: &str) {
    let path = file(dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
}

/// 文件坏了时这次不写：手改或恢复出错不能把全家的记录冲成一个人。
#[tokio::test]
async fn a_broken_file_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(Engine::unreachable_for_test(), &dir);
    write_file(&dir, "{ 坏的");
    m.note_person_languages("alice", Some("ja"));
    assert_eq!(fs::read_to_string(file(&dir)).unwrap(), "{ 坏的");

    // 修好以后照常写，主人那条还在。
    write_file(&dir, r#"{"owner": "zh"}"#);
    m.note_person_languages("alice", Some("ja"));
    let stored = read_people_languages(&file(&dir));
    assert_eq!(stored.get("alice").map(String::as_str), Some("ja"));
    assert_eq!(stored.get("owner").map(String::as_str), Some("zh"));
}

/// 文件不在是第一次写，照常建。
#[tokio::test]
async fn a_missing_file_starts_an_empty_table() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(Engine::unreachable_for_test(), &dir);
    m.note_person_languages("alice", Some("ja"));
    let stored = read_people_languages(&file(&dir));
    assert_eq!(stored.get("alice").map(String::as_str), Some("ja"));
}

/// 头在但没有能用的语言（空、只有 `*`）时不写，好的记录留着。
#[tokio::test]
async fn headers_without_usable_languages_keep_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(Engine::unreachable_for_test(), &dir);
    write_file(&dir, r#"{"owner": "zh-Hant"}"#);
    for header in ["", "*", "en;q=0"] {
        m.note_person_languages("owner", Some(header));
    }
    assert_eq!(
        read_people_languages(&file(&dir))
            .get("owner")
            .map(String::as_str),
        Some("zh-Hant")
    );
}

/// 记录真的落在这个管理器自己的 store 里。
#[test]
fn writes_go_to_the_store_of_this_manager() {
    let dir = tempfile::tempdir().unwrap();
    let m: PluginManager = manager_with(Engine::unreachable_for_test(), &dir);
    m.note_person_languages("owner", Some("zh-Hans"));
    assert_eq!(
        read_people_languages(&file(&dir))
            .get("owner")
            .map(String::as_str),
        Some("zh-Hans")
    );
}
