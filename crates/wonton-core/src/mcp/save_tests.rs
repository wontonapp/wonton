//! Bot 存文件（ADR-0027）：给定 outbox 里的文件和主人的回答，断言共享文件夹、outbox 和卡片。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{Answer, ConfirmationState};

use super::tests::{Setup, failed, setup, text};
use crate::files::test_support::Fixture;
use crate::plugin::PluginId;
use crate::plugin::docker::Engine as Docker;

const SAVE: &str = "wonton__save_file";

struct Bench {
    s: Setup,
    f: Fixture,
    outbox: PathBuf,
    id: PluginId,
}

async fn bench() -> Bench {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    let f = Fixture::new();
    s.bus.attach_files(f.files.clone());
    fs::create_dir_all(f.shares_dir.join("music")).unwrap();
    let id = PluginId::parse("com.example.agent").unwrap();
    let outbox = s.bus.plugins.app_data_dir(&id).join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    Bench { s, f, outbox, id }
}

impl Bench {
    fn offer(&self, name: &str, content: &str) {
        let path = self.outbox.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn saved(&self, path: &str) -> String {
        fs::read_to_string(self.f.shares_dir.join(path)).unwrap()
    }

    fn staged_nothing(&self) -> bool {
        let staging = self.f.dir.path().join("files/saving");
        fs::read_dir(staging).unwrap().next().is_none()
    }

    /// 发卡片的调用：回答第 `nth` 张卡片，返回结果。
    async fn answered(
        &self,
        args: Value,
        nth: usize,
        answer: Answer,
    ) -> rmcp::model::CallToolResult {
        let call = self.s.spawn(SAVE, args);
        let card = self.s.cards.wait_for(nth).await;
        self.s.answer(&card.id, answer).unwrap();
        call.await.unwrap()
    }
}

#[tokio::test]
async fn a_folder_asks_once_and_then_stays_allowed_with_its_subfolders() {
    let b = bench().await;
    b.offer("a.mp3", "A");
    b.offer("album/b.mp3", "B");
    let call = b.s.spawn(
        SAVE,
        json!({"files": ["a.mp3", "album/b.mp3"], "share": "music", "dir": "周杰伦"}),
    );
    let card = b.s.cards.wait_for(1).await;
    let answers: Vec<_> = card.choices.iter().map(|c| c.answer).collect();
    assert_eq!(
        answers,
        [Answer::Approve, Answer::ApproveWider, Answer::Reject]
    );
    assert!(
        !b.f.shares_dir.join("music/周杰伦").exists(),
        "回答之前不放"
    );
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(text(&result).contains("music/周杰伦/b.mp3"));
    assert_eq!(b.saved("music/周杰伦/a.mp3"), "A");
    assert_eq!(b.saved("music/周杰伦/b.mp3"), "B");
    assert!(!b.outbox.join("a.mp3").exists(), "存好后原文件删掉");
    assert!(b.staged_nothing());
    assert!(matches!(
        b.s.bus
            .confirmations()
            .card(&card.id)
            .map(|card| card.state),
        Some(ConfirmationState::Succeeded { .. })
    ));
    assert_eq!(b.s.bus.plugins.save_grants(&b.id), ["music/周杰伦"]);

    b.offer("c.mp3", "C");
    let args = json!({"files": ["c.mp3"], "share": "music", "dir": "周杰伦/七里香"});
    let result = b.s.call(SAVE, args).await;
    assert!(!failed(&result), "{result:?}");
    assert_eq!(b.saved("music/周杰伦/七里香/c.mp3"), "C");
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), 1, "子文件夹不再问");

    let audit = b.s.audit();
    let last = audit.last().unwrap();
    assert_eq!(last["decision"], "allowed");
    assert_eq!(last["arguments"]["matched"], "music/周杰伦");
    assert_eq!(last["arguments"]["files"][0]["size"], 1);
    assert_eq!(
        last["arguments"]["files"][0]["sha256"],
        "6b23c0d5f35d1b11f9b683f0b0a617355deb11277d91ae091d399c655b87940d"
    );
}

#[tokio::test]
async fn the_wider_button_grants_the_share_and_revoking_asks_again() {
    let b = bench().await;
    b.offer("a.mp3", "A");
    let args = json!({"files": ["a.mp3"], "share": "music", "dir": "周杰伦"});
    let result = b.answered(args, 1, Answer::ApproveWider).await;
    assert!(!failed(&result), "{result:?}");
    assert_eq!(b.s.bus.plugins.save_grants(&b.id), ["music"]);

    b.offer("b.mp3", "B");
    let args = json!({"files": ["b.mp3"], "share": "music"});
    assert!(!failed(&b.s.call(SAVE, args.clone()).await));

    b.s.bus.revoke_save_grant(&b.id, "music").unwrap();
    b.offer("c.mp3", "C");
    let call =
        b.s.spawn(SAVE, json!({"files": ["c.mp3"], "share": "music"}));
    let card = b.s.cards.wait_for(2).await;
    let answers: Vec<_> = card.choices.iter().map(|c| c.answer).collect();
    assert_eq!(
        answers,
        [Answer::Approve, Answer::Reject],
        "目标是共享文件夹本身"
    );
    b.s.answer(&card.id, Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
}

#[tokio::test]
async fn saving_never_overwrites() {
    let b = bench().await;
    fs::write(b.f.shares_dir.join("music/a.mp3"), "主人的").unwrap();
    b.offer("a.mp3", "Bot 的");
    let args = json!({"files": ["a.mp3"], "share": "music"});
    let result = b.answered(args, 1, Answer::Approve).await;
    assert!(text(&result).contains("music/a (1).mp3"), "{result:?}");
    assert_eq!(b.saved("music/a.mp3"), "主人的");
    assert_eq!(b.saved("music/a (1).mp3"), "Bot 的");
}

#[tokio::test]
async fn what_the_owner_approved_is_what_lands() {
    let b = bench().await;
    b.offer("a.mp3", "批准时的内容");
    let call =
        b.s.spawn(SAVE, json!({"files": ["a.mp3"], "share": "music"}));
    let card = b.s.cards.wait_for(1).await;
    b.offer("a.mp3", "后来换的");
    b.s.answer(&card.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
    assert_eq!(b.saved("music/a.mp3"), "批准时的内容");
    assert_eq!(
        fs::read_to_string(b.outbox.join("a.mp3")).unwrap(),
        "后来换的",
        "暂存之后写的内容不删"
    );
}

#[tokio::test]
async fn a_refusal_leaves_the_file_with_the_bot() {
    let b = bench().await;
    b.offer("a.mp3", "A");
    let args = json!({"files": ["a.mp3"], "share": "music"});
    let result = b.answered(args, 1, Answer::Reject).await;
    assert!(failed(&result) && text(&result).contains("拒绝"));
    assert!(!b.f.shares_dir.join("music/a.mp3").exists());
    assert_eq!(b.s.bus.plugins.save_grants(&b.id), Vec::<String>::new());
    assert!(b.outbox.join("a.mp3").exists());
    assert!(b.staged_nothing());
}

#[tokio::test]
async fn bad_paths_are_refused_before_any_card() {
    let b = bench().await;
    b.offer("a.mp3", "A");
    b.offer("album/.hidden", "H");
    std::os::unix::fs::symlink("/etc/hostname", b.outbox.join("link")).unwrap();
    fs::create_dir_all(b.f.shares_dir.join("music/.stfolder")).unwrap();
    let refused = [
        json!({"files": ["/etc/hostname"], "share": "music"}),
        json!({"files": ["../secrets"], "share": "music"}),
        json!({"files": ["album/.hidden"], "share": "music"}),
        json!({"files": ["link"], "share": "music"}),
        json!({"files": ["nope"], "share": "music"}),
        json!({"files": ["a.mp3", "a.mp3"], "share": "music"}),
        json!({"files": [], "share": "music"}),
        json!({"files": ["a.mp3"], "share": "没有这个"}),
        json!({"files": ["a.mp3"], "share": ".wonton-trash"}),
        json!({"files": ["a.mp3"], "share": "music", "dir": ".stfolder"}),
        json!({"files": ["a.mp3"], "share": "music", "dir": "../transfers"}),
    ];
    for args in refused {
        let result = b.s.call(SAVE, args.clone()).await;
        assert!(failed(&result), "{args}：{result:?}");
    }
    assert!(b.s.cards.posted.lock().unwrap().is_empty());
    assert!(b.staged_nothing());
}

#[tokio::test]
async fn a_symlinked_outbox_or_folder_does_not_lead_elsewhere() {
    let b = bench().await;
    b.offer("a.mp3", "A");
    let elsewhere = b.f.dir.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, b.f.shares_dir.join("music/link")).unwrap();
    b.s.bus.plugins.add_save_grant(&b.id, "music").unwrap();
    let args = json!({"files": ["a.mp3"], "share": "music", "dir": "link"});
    assert!(failed(&b.s.call(SAVE, args).await));
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);

    fs::remove_dir_all(&b.outbox).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &b.outbox).unwrap();
    fs::write(elsewhere.join("secret"), "x").unwrap();
    let args = json!({"files": ["secret"], "share": "music"});
    assert!(failed(&b.s.call(SAVE, args).await));
    assert!(!b.f.shares_dir.join("music/secret").exists());
}

#[tokio::test]
async fn a_plugin_without_an_outbox_is_told_so() {
    let b = bench().await;
    fs::remove_dir_all(&b.outbox).unwrap();
    let args = json!({"files": ["a.mp3"], "share": "music"});
    let result = b.s.call(SAVE, args).await;
    assert!(
        failed(&result) && text(&result).contains("outbox"),
        "{result:?}"
    );
}

#[tokio::test]
async fn shares_are_listed_by_name() {
    let b = bench().await;
    let result = b.s.call("wonton__list_shares", json!({})).await;
    assert_eq!(text(&result), json!({"shares": ["music"]}).to_string());
}

#[tokio::test]
async fn starting_up_clears_what_a_crash_left_in_staging() {
    let b = bench().await;
    let staging = b.f.dir.path().join("files/saving/left-over");
    fs::create_dir_all(&staging).unwrap();
    let files = b.f.reopen();
    assert!(!staging.exists(), "启动时清掉上次的暂存");
    drop(files);
}
