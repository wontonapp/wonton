//! 分享进来的行为测试（ADR-0097）：给定收件目录和一串操作，断言壳看到的分享页和交给附件区的东西。

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::json;
use wonton_protocol::{
    AttachmentKind, ChatAccount, ChatSync, ChatViewModel, CoreError, ImageUpload, PendingShare,
    PushStatus, RecoveryStatus, RoomMembership, RoomRow, ShareInDestinations, ShareInImage,
    UploadSource,
};

use super::ShareInLink;
use crate::composer::NewItem;

const TOKEN: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const BOT: &str = "@hermes:home.test";

/// 照壳的写法放一个收件目录：文件、清单，最后写 `.ready`。
fn write_inbox(root: &Path, token: &str, manifest: &serde_json::Value, files: &[(&str, &[u8])]) {
    let dir = root.join("share-inbox").join(token);
    fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in files {
        fs::write(dir.join(name), bytes).unwrap();
    }
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    fs::write(dir.join(".ready"), b"").unwrap();
}

fn photos_and_a_file(root: &Path) {
    write_inbox(
        root,
        TOKEN,
        &json!({
            "v": 1,
            "text": " 哪张好看 ",
            "items": [
                {"file": "0.jpg", "name": "IMG_0001.JPG", "mime": "image/jpeg"},
                {"file": "1", "name": "账单.pdf", "mime": "application/pdf"},
                {"file": "2.png", "name": "截图.png", "mime": "image/png"}
            ]
        }),
        &[("0.jpg", b"jpeg"), ("1", b"pdf-bytes"), ("2.png", b"png")],
    );
}

fn room(id: &str, direct: Option<&str>) -> RoomRow {
    RoomRow {
        room_id: id.into(),
        name: id.trim_start_matches('!').into(),
        is_direct: direct.is_some(),
        direct_user_id: direct.map(str::to_owned),
        plugin: None,
        is_encrypted: true,
        is_read_only: false,
        membership: RoomMembership::Joined,
        preview: None,
        side_chats: false,
        unread_count: 0,
        layout: wonton_protocol::ChatLayout::default(),
    }
}

fn chat(account: ChatAccount, sync: ChatSync, rooms: Vec<RoomRow>) -> ChatViewModel {
    ChatViewModel {
        chat_layout_default: Default::default(),
        account,
        sync,
        encryption: wonton_protocol::EncryptionStatus {
            verification: wonton_protocol::DeviceVerification::Unknown,
            recovery: RecoveryStatus::Unknown,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device: false,
            can_reset: false,
            recovery_next_step: wonton_protocol::RecoveryNextStep::None,
            verification_flow: None,
            key_handoff: None,
        },
        installing_bots: Vec::new(),
        rooms,
        problem: None,
        push: PushStatus::NoEndpoint,
        pending_confirmations: Vec::new(),
        settled_confirmations: Vec::new(),
        background_notifications: Vec::new(),
        bots: Vec::new(),
    }
}

fn signed_in() -> ChatAccount {
    ChatAccount::SignedIn {
        user_id: "@owner:home.test".into(),
        device_id: "PHONE".into(),
    }
}

/// 聊天列表：Bot 私聊、一个群、一个只读的通知私聊、一个还没接受的邀请。
fn synced_chat() -> ChatViewModel {
    let mut notices = room("!wonton", Some("@wonton:home.test"));
    notices.is_read_only = true;
    let mut invite = room("!invite", None);
    invite.membership = RoomMembership::Invited;
    chat(
        signed_in(),
        ChatSync::Running,
        vec![
            room("!family", None),
            room("!hermes", Some(BOT)),
            notices,
            invite,
        ],
    )
}

fn pending(link: &ShareInLink, paired: bool, chat: &ChatViewModel) -> Option<PendingShare> {
    let claimed = link.pending.borrow().clone();
    link.view(claimed.as_ref(), paired, chat, Some(BOT)).pending
}

fn photo(name: &str) -> ImageUpload {
    ImageUpload {
        filename: name.into(),
        mime_type: "image/jpeg".into(),
        data: vec![1, 2, 3],
        width: Some(1),
        height: Some(1),
        thumbnail: None,
    }
}

#[test]
fn a_claimed_share_shows_its_items_in_order_and_suggests_the_agent_chat() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let link = ShareInLink::new(dir.path());
    assert!(
        pending(&link, true, &synced_chat()).is_none(),
        "认领前没有分享页"
    );

    link.claim(TOKEN).unwrap();
    let share = pending(&link, true, &synced_chat()).expect("分享页出来了");
    assert_eq!(share.id, TOKEN);
    assert_eq!(share.text, "哪张好看");
    assert_eq!(share.dropped, 0);
    let names: Vec<_> = share.items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["IMG_0001.JPG", "账单.pdf", "截图.png"]);
    assert_eq!(share.items[1].kind, AttachmentKind::File);
    assert_eq!(share.items[1].size, 9);
    assert!(
        Path::new(&share.items[0].path).is_file(),
        "壳从这个路径读照片"
    );
    assert!(
        !dir.path().join("share-inbox").join(TOKEN).exists(),
        "收件目录搬走了"
    );
    let ShareInDestinations::Ready {
        rooms,
        suggested_room,
    } = share.destinations
    else {
        panic!("去向应该齐了：{:?}", share.destinations);
    };
    let ids: Vec<_> = rooms.iter().map(|room| room.room_id.as_str()).collect();
    assert_eq!(ids, ["!family", "!hermes"], "只读的通知私聊和邀请不是去向");
    assert_eq!(
        suggested_room.as_deref(),
        Some("!hermes"),
        "没发过时默认和默认 Agent 的私聊"
    );
}

#[test]
fn a_share_can_be_claimed_only_once() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();
    assert!(matches!(
        link.claim(TOKEN),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(
        pending(&link, true, &synced_chat()).is_some(),
        "重放的链接不影响已认领的"
    );
}

#[test]
fn an_unfinished_inbox_is_refused_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let inbox = dir.path().join("share-inbox").join(TOKEN);
    fs::remove_file(inbox.join(".ready")).unwrap();
    let link = ShareInLink::new(dir.path());
    assert!(matches!(
        link.claim(TOKEN),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(!inbox.exists(), "写到一半的目录删掉，不猜内容");
    assert!(pending(&link, true, &synced_chat()).is_none());
}

#[test]
fn a_manifest_listing_a_missing_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(
        dir.path(),
        TOKEN,
        &json!({"v": 1, "items": [{"file": "0.jpg", "name": "a.jpg", "mime": "image/jpeg"}]}),
        &[],
    );
    let link = ShareInLink::new(dir.path());
    assert!(matches!(
        link.claim(TOKEN),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(!dir.path().join("share-claimed").join(TOKEN).exists());
}

#[test]
fn items_over_the_limit_are_dropped_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..12).map(|index| format!("{index}.bin")).collect();
    let items: Vec<_> = names
        .iter()
        .map(|name| json!({"file": name, "name": name}))
        .collect();
    let files: Vec<(&str, &[u8])> = names
        .iter()
        .map(|name| (name.as_str(), b"x".as_slice()))
        .collect();
    write_inbox(dir.path(), TOKEN, &json!({"v": 1, "items": items}), &files);
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();
    let share = pending(&link, true, &synced_chat()).unwrap();
    assert_eq!(share.items.len(), 10);
    assert_eq!(share.dropped, 2);
}

#[test]
fn destinations_follow_pairing_and_the_chat_state() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();
    let destinations =
        |paired, chat: &ChatViewModel| pending(&link, paired, chat).unwrap().destinations;

    assert_eq!(
        destinations(false, &synced_chat()),
        ShareInDestinations::NotPaired,
        "没配对时条目照样在，去向是去配对"
    );
    assert_eq!(
        destinations(
            true,
            &chat(ChatAccount::SigningIn, ChatSync::Idle, Vec::new())
        ),
        ShareInDestinations::Loading
    );
    assert_eq!(
        destinations(true, &chat(signed_in(), ChatSync::Starting, Vec::new())),
        ShareInDestinations::Loading,
        "房间列表还没收全"
    );
    assert_eq!(
        destinations(
            true,
            &chat(
                ChatAccount::Failed {
                    reason: wonton_protocol::ChatFailure::Unauthorized,
                    message: "设备被吊销".into(),
                },
                ChatSync::Idle,
                Vec::new()
            )
        ),
        ShareInDestinations::Unavailable {
            message: "设备被吊销".into()
        }
    );
    assert_eq!(
        destinations(true, &chat(signed_in(), ChatSync::Running, Vec::new())),
        ShareInDestinations::Ready {
            rooms: Vec::new(),
            suggested_room: None
        },
        "同步完了真的没有聊天"
    );
}

#[test]
fn sending_hands_photos_and_files_over_in_order_and_remembers_the_room() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();

    let missing = link.outbound(vec![ShareInImage {
        item_id: "0.jpg".into(),
        upload: photo("photo.jpg"),
    }]);
    assert!(
        matches!(missing, Err(CoreError::InvalidInput { .. })),
        "第二张照片壳没交回"
    );

    let outbound = link
        .outbound(vec![
            ShareInImage {
                item_id: "2.png".into(),
                upload: photo("second.jpg"),
            },
            ShareInImage {
                item_id: "0.jpg".into(),
                upload: photo("first.jpg"),
            },
        ])
        .unwrap();
    let order: Vec<String> = outbound
        .items
        .iter()
        .map(|item| match item {
            NewItem::Image(image) => image.filename.clone(),
            NewItem::File {
                source: UploadSource::Path { path },
                name,
                ..
            } => {
                assert!(Path::new(path).is_file(), "文件从认领目录上传");
                name.clone()
            }
            NewItem::File { .. } => panic!("分享的文件按本机路径上传"),
        })
        .collect();
    assert_eq!(order, ["first.jpg", "账单.pdf", "second.jpg"]);
    assert!(outbound.has_files);

    link.sent(&outbound.token, outbound.has_files, "!family");
    assert!(
        pending(&link, true, &synced_chat()).is_none(),
        "发出去后分享页收起"
    );
    assert!(
        dir.path().join("share-claimed").join(TOKEN).exists(),
        "有文件时目录留给续传"
    );

    // 下一次分享默认选上次发去的房间，重启以后也一样。
    write_inbox(
        dir.path(),
        "second-share-0001",
        &json!({"v": 1, "text": "https://example.org/a"}),
        &[],
    );
    let restarted = ShareInLink::new(dir.path());
    restarted.claim("second-share-0001").unwrap();
    let ShareInDestinations::Ready { suggested_room, .. } =
        pending(&restarted, true, &synced_chat())
            .unwrap()
            .destinations
    else {
        panic!("去向应该齐了");
    };
    assert_eq!(suggested_room.as_deref(), Some("!family"));
}

#[test]
fn a_photo_only_share_is_cleaned_up_right_after_sending() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(
        dir.path(),
        TOKEN,
        &json!({"v": 1, "items": [{"file": "0.jpg", "name": "a.jpg", "mime": "image/jpeg"}]}),
        &[("0.jpg", b"jpeg")],
    );
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();
    let outbound = link
        .outbound(vec![ShareInImage {
            item_id: "0.jpg".into(),
            upload: photo("a.jpg"),
        }])
        .unwrap();
    assert!(!outbound.has_files);
    link.sent(&outbound.token, outbound.has_files, "!gone");
    assert!(!dir.path().join("share-claimed").join(TOKEN).exists());

    // 上次发去的房间不在了：退回和默认 Agent 的私聊。
    write_inbox(
        dir.path(),
        "another-share-01",
        &json!({"v": 1, "text": "hi"}),
        &[],
    );
    link.claim("another-share-01").unwrap();
    let ShareInDestinations::Ready { suggested_room, .. } =
        pending(&link, true, &synced_chat()).unwrap().destinations
    else {
        panic!("去向应该齐了");
    };
    assert_eq!(suggested_room.as_deref(), Some("!hermes"));
}

#[test]
fn discarding_items_and_the_whole_share() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();

    link.discard(Some("1")).unwrap();
    let share = pending(&link, true, &synced_chat()).unwrap();
    assert_eq!(share.items.len(), 2);
    assert!(matches!(
        link.discard(Some("1")),
        Err(CoreError::InvalidInput { .. })
    ));
    link.discard(Some("0.jpg")).unwrap();
    link.discard(Some("2.png")).unwrap();
    assert!(
        pending(&link, true, &synced_chat()).is_some(),
        "还有文字，分享页还在"
    );

    link.discard(None).unwrap();
    assert!(pending(&link, true, &synced_chat()).is_none());
    assert!(!dir.path().join("share-claimed").join(TOKEN).exists());
    assert!(matches!(
        link.discard(None),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[test]
fn a_new_share_replaces_the_unsent_one() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    let link = ShareInLink::new(dir.path());
    link.claim(TOKEN).unwrap();
    write_inbox(
        dir.path(),
        "second-share-0001",
        &json!({"v": 1, "text": "b"}),
        &[],
    );
    link.claim("second-share-0001").unwrap();
    assert_eq!(pending(&link, true, &synced_chat()).unwrap().text, "b");
    assert!(!dir.path().join("share-claimed").join(TOKEN).exists());
}

#[test]
fn shares_older_than_a_day_are_cleaned_up() {
    let dir = tempfile::tempdir().unwrap();
    photos_and_a_file(dir.path());
    write_inbox(
        dir.path(),
        "claimed-share-01",
        &json!({"v": 1, "text": "b"}),
        &[],
    );
    let link = ShareInLink::new(dir.path());
    link.claim("claimed-share-01").unwrap();

    super::inbox::clean(
        dir.path(),
        SystemTime::now() + Duration::from_secs(23 * 3600),
    );
    assert!(dir.path().join("share-inbox").join(TOKEN).exists());

    super::inbox::clean(
        dir.path(),
        SystemTime::now() + Duration::from_secs(25 * 3600),
    );
    assert!(!dir.path().join("share-inbox").join(TOKEN).exists());
    assert!(
        !dir.path()
            .join("share-claimed")
            .join("claimed-share-01")
            .exists()
    );
}
