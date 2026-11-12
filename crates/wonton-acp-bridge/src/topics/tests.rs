//! 话题模块的行为测试：给定操作序列，断言对外输出（notice、动态、冲突查询）。

use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::*;
use crate::l10n::zh;

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

fn topic(id: &str) -> SessionKey {
    SessionKey::Topic(id.to_owned())
}

fn edit_update(paths: &[&str]) -> Value {
    json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "title": "改文件",
        "kind": "edit",
        "status": "in_progress",
        "locations": paths
            .iter()
            .map(|path| json!({"path": path}))
            .collect::<Vec<_>>(),
    })
}

fn execute_update(title: &str) -> Value {
    json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-2",
        "title": title,
        "kind": "execute",
    })
}

#[test]
fn edit_paths_fall_back_to_raw_input_when_locations_are_missing() {
    let update = json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "kind": "edit",
        "rawInput": {"file_path": "/workspace/a.txt", "content": "v1"},
    });
    let claims::ToolEdit::Files(paths) = claims::parse_tool_call(&update) else {
        panic!("没有解析出文件");
    };
    assert_eq!(paths, ["/workspace/a.txt"]);
}

#[tokio::test]
async fn conflict_query_hits_within_ten_minutes_and_misses_after() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let a = topic("t-a");
    let b = topic("t-b");
    topics
        .turn_started(
            &a,
            "改登录逻辑",
            "a",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    topics
        .tool_call(&a, &edit_update(&["src/a.rs"]), at(10), zh())
        .await
        .unwrap();
    topics
        .turn_finished(&a, "改好了", at(20), zh())
        .await
        .unwrap();
    topics
        .turn_started(
            &b,
            "也碰登录逻辑",
            "b",
            "@owner:example.org",
            None,
            at(30),
            zh(),
        )
        .await
        .unwrap();

    // 一轮结束后 10 分钟内仍算它在改
    let hit = topics
        .permission_conflict(&b, &["src/a.rs".to_owned()], at(100))
        .await
        .unwrap();
    assert_eq!(hit, Some(a.clone()));
    // 10 分钟后不再命中
    let miss = topics
        .permission_conflict(&b, &["src/a.rs".to_owned()], at(20 + 600))
        .await
        .unwrap();
    assert_eq!(miss, None);
}

#[tokio::test]
async fn second_edit_of_same_file_reports_conflict_to_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let a = topic("t-a");
    let b = topic("t-b");
    topics
        .turn_started(
            &a,
            "改登录逻辑",
            "a",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    topics
        .tool_call(&a, &edit_update(&["src/a.rs"]), at(10), zh())
        .await
        .unwrap();
    topics
        .turn_started(
            &b,
            "也碰登录逻辑",
            "b",
            "@owner:example.org",
            None,
            at(20),
            zh(),
        )
        .await
        .unwrap();
    let conflict = topics
        .tool_call(&b, &edit_update(&["src/a.rs"]), at(30), zh())
        .await
        .unwrap();
    assert_eq!(
        conflict,
        Some(EditConflict {
            other: a.clone(),
            path: "src/a.rs".to_owned(),
        })
    );
    // 双方下一轮的动态里都有这笔冲突
    assert!(
        topics
            .digest_for(&a, zh())
            .unwrap()
            .contains("都改了 src/a.rs")
    );
    assert!(
        topics
            .digest_for(&b, zh())
            .unwrap()
            .contains("都改了 src/a.rs")
    );
}

#[tokio::test]
async fn topic_turn_finished_notifies_main_and_reaches_other_digests() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let main = SessionKey::Main;
    let t = topic("t1");
    topics
        .turn_started(
            &main,
            "主对话的事",
            "main",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    topics
        .turn_started(
            &t,
            "整理照片目录",
            "t",
            "@owner:example.org",
            None,
            at(10),
            zh(),
        )
        .await
        .unwrap();
    topics
        .tool_call(
            &t,
            &edit_update(&["p/a.rs", "p/b.rs", "p/c.rs"]),
            at(20),
            zh(),
        )
        .await
        .unwrap();

    let notice = topics
        .turn_finished(
            &t,
            "整理好了 120 张照片\n排除了重复项\n生成了索引\n（完）",
            at(30),
            zh(),
        )
        .await
        .unwrap()
        .unwrap();
    // notice 的关键字段：话题名、文件数
    assert!(notice.contains("「整理照片目录」"));
    assert!(notice.contains("3 个文件"));

    // 主对话下一轮的动态里有它
    let digest = topics.digest_for(&main, zh()).unwrap();
    assert!(digest.contains("「整理照片目录」已完成"));
    assert!(digest.contains("整理好了 120 张照片"));
    assert!(digest.contains("改了：p/a.rs、p/b.rs、p/c.rs"));

    // 自己的动态里没有自己的结论；主对话还在进行中，以“正在做”出现
    topics
        .turn_started(
            &t,
            "再来一件事",
            "t",
            "@owner:example.org",
            None,
            at(40),
            zh(),
        )
        .await
        .unwrap();
    let own = topics.digest_for(&t, zh()).unwrap();
    assert!(!own.contains("整理好了 120 张照片"));
    assert!(own.contains("「主对话」正在做"));

    // 主对话的一轮结束不产生 notice
    assert_eq!(
        topics
            .turn_finished(&main, "好的", at(50), zh())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn fourth_topic_queues_and_gets_promoted_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let t1 = topic("t1");
    let t2 = topic("t2");
    let t3 = topic("t3");
    let t4 = topic("t4");
    for (key, prompt) in [(&t1, "第一件事"), (&t2, "第二件事"), (&t3, "第三件事")] {
        assert_eq!(
            topics
                .turn_started(
                    key,
                    prompt,
                    key.storage_key(),
                    "@owner:example.org",
                    None,
                    at(10),
                    zh()
                )
                .await
                .unwrap(),
            TurnStart::Started
        );
    }
    assert_eq!(
        topics
            .turn_started(
                &t4,
                "第四件事",
                "t4",
                "@owner:example.org",
                None,
                at(20),
                zh()
            )
            .await
            .unwrap(),
        TurnStart::Queued
    );
    // 没有空位不放行
    assert_eq!(topics.poll_admission(at(30)).await.unwrap(), None);
    // 排队中的话题再来一句：仍在排队，排队的话换成最新一句
    assert_eq!(
        topics
            .turn_started(
                &t4,
                "改先做这个",
                "t4",
                "@owner:example.org",
                None,
                at(40),
                zh()
            )
            .await
            .unwrap(),
        TurnStart::Queued
    );
    // 一个话题结束，名额空出，按排队先后放行
    assert!(topics.end(&t1, at(50), zh()).await.unwrap());
    assert_eq!(
        topics.poll_admission(at(60)).await.unwrap(),
        Some((
            t4.clone(),
            "改先做这个".to_owned(),
            "t4".to_owned(),
            "@owner:example.org".to_owned(),
            None
        ))
    );
    // 名额又占满，不再放行
    assert_eq!(topics.poll_admission(at(70)).await.unwrap(), None);
}

#[tokio::test]
async fn idle_topic_is_swept_and_main_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let main = SessionKey::Main;
    let t = topic("t");
    topics
        .turn_started(
            &main,
            "主对话的事",
            "main",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    topics
        .turn_started(&t, "话题的事", "t", "@owner:example.org", None, at(0), zh())
        .await
        .unwrap();
    topics
        .turn_finished(&t, "做完了", at(30), zh())
        .await
        .unwrap();

    assert!(
        topics
            .sweep(
                at(1829),
                Duration::from_millis(limits::IDLE_TOPIC_TTL_MS),
                zh()
            )
            .await
            .unwrap()
            .is_empty()
    );
    let ended = topics
        .sweep(
            at(1830),
            Duration::from_millis(limits::IDLE_TOPIC_TTL_MS),
            zh(),
        )
        .await
        .unwrap();
    assert_eq!(ended, vec![t.clone()]);
    // 结论留在工作板上，别的会话的动态里能看到它结束的原因
    let digest = topics.digest_for(&main, zh()).unwrap();
    assert!(digest.contains("「话题的事」已完成：做完了"));
    assert!(digest.contains("已结束（空闲超过 30 分钟）"));
    // 已结束的话题再 end 返回 false；主对话不参与空闲结束
    assert!(!topics.end(&t, at(1831), zh()).await.unwrap());
    assert!(
        topics
            .sweep(
                at(120 * 60),
                Duration::from_millis(limits::IDLE_TOPIC_TTL_MS),
                zh()
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn board_survives_restart_and_corruption_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("topics.json");
    let t = topic("t");
    {
        let mut topics = Topics::open(&path, at(0)).await.unwrap();
        topics
            .turn_started(
                &SessionKey::Main,
                "主提示",
                "main",
                "@owner:example.org",
                None,
                at(1),
                zh(),
            )
            .await
            .unwrap();
        topics
            .turn_started(
                &t,
                "话题第一句",
                "t",
                "@owner:example.org",
                None,
                at(2),
                zh(),
            )
            .await
            .unwrap();
        topics
            .tool_call(&t, &edit_update(&["x.rs"]), at(3), zh())
            .await
            .unwrap();
        topics
            .turn_finished(&t, "结论行", at(4), zh())
            .await
            .unwrap();
    }
    let mut reopened = Topics::open(&path, at(5)).await.unwrap();
    // 行、结论和变化都恢复
    let digest = reopened.digest_for(&SessionKey::Main, zh()).unwrap();
    assert!(digest.contains("「话题第一句」已完成"));
    assert!(digest.contains("结论行"));
    // 改文件记录也恢复：一轮结束后 10 分钟内仍算在改
    let t2 = topic("t2");
    reopened
        .turn_started(
            &t2,
            "重启后也改 x.rs",
            "t2",
            "@owner:example.org",
            None,
            at(6),
            zh(),
        )
        .await
        .unwrap();
    let hit = reopened
        .permission_conflict(&t2, &["x.rs".to_owned()], at(100))
        .await
        .unwrap();
    assert_eq!(hit, Some(t.clone()));
    // 序号在重启后继续单调，新变化排得上号
    reopened
        .tool_call(&t2, &execute_update("跑测试"), at(7), zh())
        .await
        .unwrap();
    assert!(
        reopened
            .digest_for(&SessionKey::Main, zh())
            .unwrap()
            .contains("执行了「跑测试」")
    );
    // 文件损坏报 Upstream，不静默清空
    std::fs::write(&path, b"not json").unwrap();
    let error = Topics::open(&path, at(8)).await.unwrap_err();
    assert!(matches!(error, crate::error::Error::Upstream(_)));
}

#[tokio::test]
async fn execute_is_noted_on_board_but_claims_no_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let t = topic("t");
    topics
        .turn_started(&t, "跑个命令", "t", "@owner:example.org", None, at(0), zh())
        .await
        .unwrap();
    topics
        .tool_call(&t, &execute_update("ls -la"), at(1), zh())
        .await
        .unwrap();
    // 不记文件：notice 里没有文件数
    let notice = topics
        .turn_finished(&t, "跑完了", at(2), zh())
        .await
        .unwrap()
        .unwrap();
    assert!(notice.contains("「跑个命令」"));
    assert!(!notice.contains("个文件"));
    // 工作板上有一笔，下一个开新轮的会话（这里还没有行，从工作板开头看起）能看到
    let other = topic("other");
    let digest = topics.digest_for(&other, zh()).unwrap();
    assert!(digest.contains("执行了「ls -la」"));
    assert!(digest.contains("「跑个命令」已完成"));
    topics
        .turn_started(
            &other,
            "别的事",
            "other",
            "@owner:example.org",
            None,
            at(3),
            zh(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn opening_carries_context_board_rules_and_first_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let main = SessionKey::Main;
    let other = topic("other");
    let t = topic("t");
    topics
        .turn_started(
            &main,
            "主对话正在做的事",
            "main",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    topics
        .turn_started(
            &other,
            "别的话题的事",
            "other",
            "@owner:example.org",
            None,
            at(1),
            zh(),
        )
        .await
        .unwrap();

    let preceding: Vec<String> = (1..=8).map(|index| format!("主对话消息{index}")).collect();
    // 真实顺序：opening 在话题自己 turn_started 之前调，板上还没有自己
    let opening = topics.opening_for(&t, "整理照片目录并生成索引", &preceding, zh());
    // 起点那条加它前面最多 6 条：1、2 被截掉，3 以后都在
    assert!(opening.contains("主对话的片段（仅供参考）"));
    assert!(opening.contains("主对话消息3"));
    assert!(!opening.contains("主对话消息2"));
    assert!(opening.contains("- 整理照片目录并生成索引"));
    // 当前工作板：别的会话都在，自己还没入场
    assert!(opening.contains("当前工作板"));
    assert!(opening.contains("「主对话」进行中：主对话正在做的事"));
    assert!(opening.contains("「别的话题的事」进行中"));
    assert!(!opening.contains("「整理照片目录并生成索引」进行中"));
    // 分工规矩原话和用户的第一句
    assert!(opening.contains("只负责这个话题里用户要的事"));
    assert!(opening.contains("用户的第一句：整理照片目录并生成索引"));
}

#[tokio::test]
async fn topic_name_takes_first_twenty_characters() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let t = topic("t");
    let prompt = "这是一个超过二十个字符的话题第一句话，用来验证名字按字符截短而不是按字节截短";
    topics
        .turn_started(&t, prompt, "t", "@owner:example.org", None, at(0), zh())
        .await
        .unwrap();
    let notice = topics
        .turn_finished(&t, "done", at(1), zh())
        .await
        .unwrap()
        .unwrap();
    let expected: String = prompt.chars().take(20).collect();
    assert!(notice.contains(&format!("话题「{expected}」：已完成")));
}

#[tokio::test]
async fn digest_lists_at_most_five_recent_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let main = SessionKey::Main;
    topics
        .turn_started(
            &main,
            "主对话的事",
            "main",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    let t = topic("t");
    for index in 1..=7 {
        topics
            .turn_started(
                &t,
                &format!("第{index}件事"),
                "t",
                "@owner:example.org",
                None,
                at(index),
                zh(),
            )
            .await
            .unwrap();
        topics
            .turn_finished(&t, &format!("结论{index}"), at(index), zh())
            .await
            .unwrap();
    }
    let digest = topics.digest_for(&main, zh()).unwrap();
    // 只列最近 5 条变化
    assert!(digest.contains("结论7"));
    assert!(digest.contains("结论3"));
    assert!(!digest.contains("结论2"));
    // 表头 + 5 条变化（话题空闲，没有“正在做”的行）
    assert_eq!(digest.lines().count(), 6);
}

#[tokio::test]
async fn digest_is_none_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let main = SessionKey::Main;
    topics
        .turn_started(
            &main,
            "主对话的事",
            "main",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    assert!(topics.digest_for(&main, zh()).is_none());
}

#[tokio::test]
async fn permission_request_marks_waiting_and_settles_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let a = topic("t-a");
    let b = topic("t-b");
    topics
        .turn_started(
            &a,
            "改登录逻辑",
            "a",
            "@owner:example.org",
            None,
            at(0),
            zh(),
        )
        .await
        .unwrap();
    topics
        .tool_call(&a, &edit_update(&["src/a.rs"]), at(5), zh())
        .await
        .unwrap();
    topics
        .turn_started(
            &b,
            "也碰登录逻辑",
            "b",
            "@owner:example.org",
            None,
            at(10),
            zh(),
        )
        .await
        .unwrap();
    let conflict = topics
        .permission_conflict(&b, &["src/a.rs".to_owned()], at(20))
        .await
        .unwrap();
    assert_eq!(conflict, Some(a.clone()));
    // 别的会话眼里：b 在等确认
    assert!(topics.digest_for(&a, zh()).unwrap().contains("等你确认"));
    // 卡片有了结果，放回进行中
    topics.permission_settled(&b).await.unwrap();
    assert!(
        topics
            .digest_for(&a, zh())
            .unwrap()
            .contains("「也碰登录逻辑」正在做")
    );
}

#[tokio::test]
async fn ended_topic_reopens_on_new_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut topics = Topics::open(dir.path().join("topics.json"), at(0))
        .await
        .unwrap();
    let t = topic("t");
    topics
        .turn_started(&t, "第一件事", "t", "@owner:example.org", None, at(0), zh())
        .await
        .unwrap();
    topics
        .turn_finished(&t, "做完了", at(1), zh())
        .await
        .unwrap();
    assert!(topics.end(&t, at(2), zh()).await.unwrap());
    // 已结束的话题又来消息：重新按入场规则开始，名字保持话题第一条消息
    assert_eq!(
        topics
            .turn_started(
                &t,
                "又来一件事",
                "t",
                "@owner:example.org",
                None,
                at(3),
                zh()
            )
            .await
            .unwrap(),
        TurnStart::Started
    );
    let notice = topics
        .turn_finished(&t, "又做完了", at(4), zh())
        .await
        .unwrap()
        .unwrap();
    assert!(notice.contains("「第一件事」"));
}
