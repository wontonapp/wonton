mod own_private_bot_view_tests;

use serde::de::DeserializeOwned;

use super::*;
use crate::{
    Action, CoreError, InstallProgress, InstallStage, NotificationCategory, PluginFailure,
    PluginOperation, PushContent, PushNotification, PushStatus, SecretString,
};

/// 测试用的恢复密钥原文。
const KEY: &str = "EsTc 1234 abcd";

fn sample_view_model() -> ChatViewModel {
    ChatViewModel {
        ask: None,
        chat_layout_default: Default::default(),
        account: ChatAccount::SignedIn {
            user_id: "@owner:home.example".into(),
            device_id: "ABCDEFGH".into(),
        },
        sync: ChatSync::Failed {
            reason: ChatFailure::TransportUnavailable,
            message: "连不上 Matrix 服务器".into(),
        },
        encryption: EncryptionStatus {
            verification: DeviceVerification::Unverified,
            recovery: RecoveryStatus::Incomplete,
            recovery_key: Some(SecretString::new(KEY)),
            backup_reminder: Some(BackupReminder::SaveRecoveryKey),
            can_verify_with_other_device: true,
            can_reset: true,
            recovery_next_step: RecoveryNextStep::ChooseDeviceOrKey,
            verification_flow: None,
            key_handoff: None,
        },
        installing_bots: vec![BotInstallRow {
            plugin_id: "app.hermes".into(),
            title: "Hermes".into(),
            icon: None,
            operation: PluginOperation::Install,
            source: Some("github:example/hermes@v1.2.0".into()),
            state: BotInstallState::Running {
                progress: InstallProgress {
                    stage: InstallStage::PullingImages,
                    percent: Some(40),
                },
            },
        }],
        rooms: vec![RoomRow {
            unread_mentions: 0,
            layout: Default::default(),
            call_active: false,
            room_id: "!bot:home.example".into(),
            name: "Hermes".into(),
            is_direct: true,
            direct_user_id: Some("@hermes:home.example".into()),
            plugin: None,
            bot_chat: None,
            bot: None,
            is_encrypted: true,
            is_read_only: false,
            membership: RoomMembership::Invited,
            preview: Some(RoomPreview {
                sender_name: "Hermes".into(),
                is_own: false,
                body: MessageSummary::Image,
                timestamp_ms: 1_793_000_000_000,
            }),
            side_chats: false,
            unread_count: 2,
        }],
        shared_contacts: Vec::new(),
        problem: Some(ChatProblem {
            reason: ChatFailure::Unauthorized,
            message: "加入房间失败".into(),
        }),
        push: PushStatus::Registering,
        pending_confirmations: vec![PendingConfirmation {
            id: "card".into(),
            room_id: Some("!bot:home.example".into()),
            title: "Hermes 想安装 文件搜索".into(),
            requester: None,
            expires_at_ms: 1_793_000_000_000,
            ongoing: false,
        }],
        settled_confirmations: vec!["older-card".into()],
        background_notifications: vec![PushNotification {
            server_id: Some("a1".into()),
            room_id: "!bot:home.example".into(),
            event_id: "$reply".into(),
            sender: Some("Hermes".into()),
            room_name: None,
            content: PushContent::Message {
                summary: MessageSummary::Text {
                    text: "好了".into(),
                },
            },
            category: NotificationCategory::Message,
            confirmation: None,
            contact_request: None,
            thread_root: None,
            hidden: false,
            lock_screen_private: true,
            unread_total: 2,
        }],
        bots: Vec::new(),
        own_private_bot: None,
        own_private_bot_problem: None,
    }
}

#[test]
fn a_person_contact_omits_the_unknown_access() {
    let contact = ChatContact {
        user_id: "@bob:home".into(),
        name: "Bob".into(),
        bot: None,
        side_chat_room_id: None,
        access: crate::BotAccess::Unknown,
    };
    let wire = serde_json::to_value(&contact).unwrap();
    assert!(wire.get("access").is_none());
    // 旧解析端缺字段按默认 Unknown，和新端一致。
    let back: ChatContact = serde_json::from_value(wire).unwrap();
    assert_eq!(back.access, crate::BotAccess::Unknown);
}

/// 契约：聊天的 ViewModel、操作和错误都能经 JSON 往返，供将来的 Web 端经 WebSocket 收发。
#[test]
fn chat_types_round_trip_through_json() {
    let view_model = sample_view_model();
    assert_eq!(round_trip(&view_model), view_model);

    let image = ImageUpload {
        filename: "cat.jpg".into(),
        mime_type: "image/jpeg".into(),
        data: vec![0xff; 4096],
        width: Some(640),
        height: None,
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/jpeg".into(),
            data: vec![0xee; 2048],
            width: 320,
            height: 240,
        }),
    };
    let actions = [
        Action::Chat {
            action: crate::ChatAction::JoinRoom {
                room_id: "!a:home.example".into(),
            },
        },
        Action::Chat {
            action: crate::ChatAction::StartDirectChat {
                user_id: "@hermes:home.example".into(),
            },
        },
        Action::Timeline {
            action: crate::TimelineAction::SendImage {
                target: crate::TimelineTarget {
                    room_id: "!a:home.example".into(),
                    thread_root: None,
                },
                caption: None,
                image: image.clone(),
                reply_to: None,
            },
        },
        Action::Chat {
            action: crate::ChatAction::RecoverEncryption {
                recovery_key: SecretString::new(KEY),
            },
        },
        Action::Chat {
            action: crate::ChatAction::ConfirmRecoveryKeySaved,
        },
        Action::Chat {
            action: crate::ChatAction::DismissChatProblem,
        },
    ];
    for action in &actions {
        assert_eq!(&round_trip(action), action);
    }
    // 恢复密钥能解开主人全部的加密历史：ViewModel 和操作打进日志时都不能带出原文。
    assert!(
        !format!("{view_model:?}").contains(KEY),
        "ViewModel 的 Debug 漏了恢复密钥"
    );
    assert!(
        !format!("{actions:?}").contains(KEY),
        "操作的 Debug 漏了恢复密钥"
    );
    assert_eq!(
        serde_json::to_value(SecretString::new(KEY)).unwrap(),
        serde_json::json!(KEY),
        "序列化时是原文（Web 端要用）"
    );
    let error = CoreError::TransportUnavailable {
        reason: "隧道没连上".into(),
    };
    assert_eq!(round_trip(&error), error);
    let printed = format!("{image:?}");
    assert!(
        !printed.contains("255, 255") && !printed.contains("238, 238"),
        "Debug 不能输出原图和缩略图的字节：{printed}"
    );
}

fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

/// 旧核心没有安装行；新核心的阶段旧 App 不认识时落到 `Other`，行照样显示。
#[test]
fn install_rows_fall_back_on_old_cores_and_new_stages() {
    let mut view = serde_json::to_value(sample_view_model()).unwrap();
    view.as_object_mut().unwrap().remove("installing_bots");
    let view: ChatViewModel = serde_json::from_value(view).unwrap();
    assert!(view.installing_bots.is_empty());

    let stage: InstallStage = serde_json::from_value(serde_json::json!("migrating")).unwrap();
    assert_eq!(stage, InstallStage::Other);
    let state: BotInstallState = serde_json::from_value(serde_json::json!({
        "state": "running",
        "progress": {"stage": "migrating", "percent": null},
    }))
    .unwrap();
    assert_eq!(
        state,
        BotInstallState::Running {
            progress: InstallProgress {
                stage: InstallStage::Other,
                percent: None,
            }
        }
    );
    assert_eq!(
        serde_json::to_value(BotInstallState::Failed {
            reason: PluginFailure::Upstream,
            message: "Docker 报错".into(),
        })
        .unwrap(),
        serde_json::json!({"state": "failed", "reason": "upstream", "message": "Docker 报错"})
    );
}

/// 契约：备份页的下一步类别都能经 JSON 往返；新核心的新类别落 `Other`，旧核心没有这两个字段时
/// `can_reset` 落假、下一步落 `None`，壳不画重置也不画下一步。
#[test]
fn recovery_next_step_falls_back_on_old_cores_and_new_categories() {
    for step in [
        RecoveryNextStep::EnterKey,
        RecoveryNextStep::ChooseDeviceOrKey,
        RecoveryNextStep::ShowFlow,
        RecoveryNextStep::Progress,
        RecoveryNextStep::None,
    ] {
        assert_eq!(round_trip(&step), step);
    }
    assert_eq!(
        serde_json::from_value::<RecoveryNextStep>(serde_json::json!("recalibrating")).unwrap(),
        RecoveryNextStep::Other
    );
    let mut value = serde_json::to_value(sample_view_model().encryption).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("can_reset");
    object.remove("recovery_next_step");
    let status: EncryptionStatus = serde_json::from_value(value).unwrap();
    assert!(!status.can_reset);
    assert_eq!(status.recovery_next_step, RecoveryNextStep::None);
}

/// 契约：跨服务器私聊失败的两个原因各有自己的取值，不并进 `Forbidden`/`Other`；
/// 不认识的取值才落 `Other`。
#[test]
fn cross_server_problems_keep_their_category() {
    for (wire, expected) in [
        ("server_not_paired", ChatFailure::ServerNotPaired),
        ("peer_public_access_off", ChatFailure::PeerPublicAccessOff),
    ] {
        let problem: ChatProblem = serde_json::from_value(serde_json::json!({
            "reason": wire,
            "message": "发起私聊失败",
        }))
        .unwrap();
        assert_eq!(problem.reason, expected);
        assert_eq!(
            serde_json::to_value(expected).unwrap(),
            serde_json::json!(wire)
        );
        assert_eq!(round_trip(&problem), problem);
    }
    assert_eq!(
        serde_json::from_value::<ChatFailure>(serde_json::json!("hibernating")).unwrap(),
        ChatFailure::Other
    );
}
