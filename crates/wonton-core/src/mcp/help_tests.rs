//! 帮助文档的行为：镜像里的文档、取新版、取不到时旧的照常用、版本门槛、语言。官方索引用本地目录。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use wonton_l10n::Arg;

use super::tests::{failed, manager, setup, text};
use crate::help::{CoreVersion, Help};
use crate::l10n::CATALOG;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::languages::Languages;
use crate::plugin::package_files::sha256_hex;

const BASELINE: &str = "1111111111111111111111111111111111111111";
const NEWER: &str = "2222222222222222222222222222222222222222";

fn page(title: &str, extra: &str, body: &str) -> String {
    format!("---\ntitle: {title}\nsummary: {title}的说明\n{extra}---\n\n{body}\n")
}

fn version(version: Option<&str>) -> CoreVersion {
    let version = version.map(str::to_owned);
    Arc::new(move || {
        let version = version.clone();
        Box::pin(async move { version })
    })
}

struct Rig {
    dir: tempfile::TempDir,
    manager: Arc<PluginManager>,
}

impl Rig {
    /// 镜像里的文档有 `devices` 的中英文和只有英文的 `models`。官方索引没有 `docs`。
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(Engine::unreachable_for_test(), &dir);
        let rig = Self { dir, manager };
        rig.write("image/user/zh/devices.md", &page("加设备", "", "旧的正文"));
        rig.write("image/user/en/devices.md", &page("Devices", "", "old body"));
        rig.write(
            "image/user/en/models.md",
            &page("Models", "", "models body"),
        );
        rig.write(
            "image/source.json",
            &json!({"commit": BASELINE, "updated_at_ms": 1000}).to_string(),
        );
        rig.write_index(&json!({"format": 1, "plugins": []}));
        rig
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn write_index(&self, index: &Value) {
        self.write("index/index.json", &index.to_string());
    }

    /// 官方索引指向 `commit`，内容是 `pages`。`blobs` 为假时索引目录里没有文件，相当于取到一半断网。
    fn publish(&self, commit: &str, min_core: &str, pages: &[(&str, &str)], blobs: bool) {
        let mut files = serde_json::Map::new();
        for (path, text) in pages {
            let sha256 = sha256_hex(text.as_bytes());
            if blobs {
                self.write(&format!("index/blobs/{sha256}"), text);
            }
            files.insert((*path).to_owned(), json!(sha256));
        }
        self.write_index(&json!({
            "format": 1,
            "plugins": [],
            "docs": {
                "source": "github:wontonapp/wonton-docs",
                "commit": commit,
                "min_core": min_core,
                "files": files,
            },
        }));
    }

    fn help(&self, core: Option<&str>) -> Arc<Help> {
        Help::new(
            self.dir.path().join("image"),
            self.manager.clone(),
            version(core),
        )
    }

    fn cached(&self, commit: &str) -> bool {
        Path::new(&self.manager.docs_cache()).join(commit).is_dir()
    }
}

#[tokio::test]
async fn the_docs_in_the_image_answer_without_an_index_entry() {
    let rig = Rig::new();
    let help = rig.help(None);
    let toc = help.answer(None, Some("zh")).await.unwrap();
    assert_eq!(toc["core_version"], "unknown");
    assert_eq!(toc["docs_commit"], BASELINE);
    assert!(toc.get("refresh_problem").is_none(), "{toc}");
    let topics = toc["topics"].as_array().unwrap();
    assert_eq!(topics.len(), 2);
    assert_eq!(topics[0]["topic"], "devices");
    assert_eq!(topics[0]["title"], "加设备");
    assert_eq!(topics[1]["lang"], "en", "缺这种语言时回退，和插件文字一样");

    let body = help.answer(Some("devices"), Some("zh")).await.unwrap();
    assert_eq!(body["body"], "旧的正文\n");
}

#[tokio::test]
async fn an_unknown_topic_lists_what_can_be_asked() {
    let rig = Rig::new();
    let error = rig
        .help(None)
        .answer(Some("add device"), None)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    let message = error.to_string();
    assert!(
        message.contains("devices") && message.contains("models"),
        "{message}"
    );
}

#[tokio::test]
async fn no_docs_at_all_is_reported() {
    let rig = Rig::new();
    fs::remove_dir_all(rig.dir.path().join("image")).unwrap();
    let error = rig.help(None).answer(None, None).await.unwrap_err();
    assert_eq!(error.kind(), "docs_unavailable");
}

#[tokio::test]
async fn newer_docs_replace_the_old_ones_and_survive_a_restart() {
    let rig = Rig::new();
    let text = page("加设备", "", "新的正文");
    rig.publish(NEWER, "0.1.0", &[("user/zh/devices.md", &text)], true);
    let help = rig.help(Some("0.9.0"));
    let body = help.answer(Some("devices"), Some("zh")).await.unwrap();
    assert_eq!(body["docs_commit"], NEWER);
    assert_eq!(body["core_version"], "0.9.0");
    assert_eq!(body["body"], "新的正文\n");
    assert_eq!(body["docs_updated_days_ago"], 0);

    rig.write_index(&json!({"format": 1, "plugins": []}));
    let restarted = rig.help(Some("0.9.0"));
    let toc = restarted.answer(None, None).await.unwrap();
    assert_eq!(toc["docs_commit"], NEWER, "取到的比镜像里的新");
}

#[tokio::test]
async fn an_interrupted_fetch_keeps_the_old_docs_and_says_why() {
    let rig = Rig::new();
    let good = page("加设备", "", "新的正文");
    let lost = page("Devices", "", "new body");
    rig.publish(
        NEWER,
        "0.1.0",
        &[("user/zh/devices.md", &good), ("user/en/devices.md", &lost)],
        true,
    );
    let blobs = rig.dir.path().join("index/blobs");
    fs::remove_file(blobs.join(sha256_hex(lost.as_bytes()))).unwrap();

    let body = rig
        .help(None)
        .answer(Some("devices"), Some("zh"))
        .await
        .unwrap();
    assert_eq!(body["docs_commit"], BASELINE);
    assert_eq!(body["body"], "旧的正文\n");
    // refresh_problem 整段用主人的语言（rig 没记主人的语言，回退英文）：取新版失败的原因
    // 在前面（细节带临时目录，原样带上），后面由目录写文档的年龄。期望值由目录格式化。
    let problem = body["refresh_problem"].as_str().unwrap();
    let owner = CATALOG.negotiate(Languages::from_header(None).requested());
    let failed = CATALOG.format(owner, "chat-help-refresh-failed", &[("details", "".into())]);
    assert!(problem.starts_with(failed.trim_end()), "{problem}");
    let blob_prefix = CATALOG
        .format(
            owner,
            "plugin-settings-store-index-blob-missing",
            &[("path", "".into()), ("error", "".into())],
        )
        .split(" has no ")
        .next()
        .unwrap_or("")
        .to_owned();
    assert!(problem.contains(&blob_prefix), "{problem}");
    let days = body["docs_updated_days_ago"].as_u64().unwrap();
    let age = CATALOG.format(
        owner,
        "chat-help-docs-age",
        &[("problem", "".into()), ("days", Arg::from(days))],
    );
    assert!(problem.ends_with(&age), "{problem}");
    assert!(!rig.cached(NEWER), "没取全不切换");
}

#[tokio::test]
async fn a_file_that_does_not_match_its_hash_is_not_used() {
    let rig = Rig::new();
    let text = page("加设备", "", "新的正文");
    rig.publish(NEWER, "0.1.0", &[("user/zh/devices.md", &text)], true);
    let blob = rig
        .dir
        .path()
        .join("index/blobs")
        .join(sha256_hex(text.as_bytes()));
    fs::write(blob, page("加设备", "", "被改过")).unwrap();
    let body = rig.help(None).answer(None, None).await.unwrap();
    assert_eq!(body["docs_commit"], BASELINE);
    assert!(
        body["refresh_problem"].as_str().unwrap().contains("sha256"),
        "{body}"
    );
}

#[tokio::test]
async fn a_path_outside_the_docs_stops_the_fetch() {
    let rig = Rig::new();
    let text = page("加设备", "", "新的正文");
    rig.publish(NEWER, "0.1.0", &[("user/../../evil.md", &text)], true);
    let body = rig.help(None).answer(None, None).await.unwrap();
    assert_eq!(body["docs_commit"], BASELINE);
    assert!(
        body["refresh_problem"].as_str().unwrap().contains("路径"),
        "{body}"
    );
    assert!(!rig.manager.docs_cache().join("../../evil.md").exists());
}

#[tokio::test]
async fn docs_that_need_a_newer_core_are_not_switched_to() {
    let rig = Rig::new();
    let text = page("加设备", "", "新的正文");
    rig.publish(NEWER, "1.0.0", &[("user/zh/devices.md", &text)], true);
    let body = rig.help(Some("0.9.0")).answer(None, None).await.unwrap();
    assert_eq!(body["docs_commit"], BASELINE);
    let problem = body["refresh_problem"].as_str().unwrap();
    assert!(
        problem.contains("1.0.0") && problem.contains("0.9.0"),
        "{problem}"
    );
    assert!(!rig.cached(NEWER));

    let unversioned = rig.help(None).answer(None, None).await.unwrap();
    assert_eq!(
        unversioned["docs_commit"], NEWER,
        "开发构建没有版本号，不比较"
    );
}

#[tokio::test]
async fn a_page_for_a_newer_core_says_so() {
    let rig = Rig::new();
    let text = page("备份", "requires_core: 1.2.0\n", "正文");
    rig.publish(NEWER, "0.1.0", &[("user/zh/backup.md", &text)], true);
    let body = rig
        .help(Some("0.9.0"))
        .answer(Some("backup"), Some("zh"))
        .await
        .unwrap();
    assert_eq!(body["requires_core"], "1.2.0");
    assert!(body["note"].as_str().unwrap().contains("1.2.0"), "{body}");
}

#[tokio::test]
async fn the_default_language_is_the_one_the_app_asked_for() {
    let rig = Rig::new();
    let help = rig.help(None);
    let before = help.answer(Some("devices"), None).await.unwrap();
    assert_eq!(before["lang"], "en");
    rig.manager
        .note_person_languages("owner", Some("zh-Hans-CN, en;q=0.8"));
    let after = help.answer(Some("devices"), None).await.unwrap();
    assert_eq!(after["lang"], "zh");
}

/// 取索引卡住时不让工具一直等：用手头的文档回答。
#[tokio::test(start_paused = true)]
async fn a_slow_refresh_does_not_hold_up_the_answer() {
    let rig = Rig::new();
    let calls = Arc::new(AtomicU32::new(0));
    let counted = calls.clone();
    let slow_version: CoreVersion = Arc::new(move || {
        let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move {
            if first {
                tokio::time::sleep(Duration::from_secs(600)).await;
            }
            None
        })
    });
    let help = Help::new(
        rig.dir.path().join("image"),
        rig.manager.clone(),
        slow_version,
    );
    let toc = help.answer(None, None).await.unwrap();
    assert_eq!(toc["docs_commit"], BASELINE);
}

#[tokio::test]
async fn agents_reach_the_docs_through_the_bus() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let missing = s.call("wonton__help", json!({})).await;
    assert!(failed(&missing));
    let meta = missing.meta.as_ref().unwrap();
    assert_eq!(meta.0["app.wonton/failure"], "service_unavailable");

    let rig = Rig::new();
    s.bus.attach_help(rig.help(None));
    let page = s
        .call("wonton__help", json!({"topic": "devices", "lang": "en"}))
        .await;
    assert!(!failed(&page), "{}", text(&page));
    let page: Value = serde_json::from_str(&text(&page)).unwrap();
    assert_eq!(page["body"], "old body\n");
}
