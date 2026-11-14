use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::*;
use crate::{Action, RoomRow};

const ID: &str = "aaaqeayeaudaocajbifqydiob4";
const OTHER_ID: &str = "77777777777777777777777774";

fn id() -> FileId {
    ID.parse().unwrap()
}

fn other_id() -> FileId {
    OTHER_ID.parse().unwrap()
}

fn link() -> String {
    format!("wonton://file/{ID}")
}

fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

/// 契约：编号是 RFC 4648 的小写 base32，向量由 Python 的 `base64.b32encode` 算出。
#[test]
fn file_id_encodes_128_bits_as_lowercase_base32() {
    let bytes: [u8; 16] = std::array::from_fn(|index| index as u8);
    assert_eq!(FileId::from_bytes(bytes).as_str(), ID);
    assert_eq!(FileId::from_bytes([0xff; 16]).as_str(), OTHER_ID);
    assert_eq!(FileId::from_bytes([0; 16]).as_str(), "a".repeat(26));
}

/// 契约：链接只有 `wonton://file/<编号>` 这一段。聊天记录里存的就是它，改了旧消息就认不出来了。
#[test]
fn link_format_is_stable() {
    assert_eq!(id().link(), format!("wonton://file/{ID}"));
    assert_eq!(FileId::from_link(&id().link()), Some(id()));
    for bad in [
        ID.to_owned(),
        format!("wonton://file/{ID} "),
        format!(" wonton://file/{ID}"),
        format!("wonton://file/{ID}/"),
        format!("WONTON://file/{ID}"),
        format!("wonton://files/{ID}"),
    ] {
        assert_eq!(FileId::from_link(&bad), None, "{bad}");
    }
}

/// 编号会拼进 URL 路径，从 JSON 和字符串来的都要校验。
#[test]
fn file_id_rejects_anything_but_26_base32_characters() {
    for bad in [
        String::new(),
        "a".repeat(25),
        "a".repeat(27),
        ID.to_uppercase(),
        format!("{}1", "a".repeat(25)),
        format!("{}/", "a".repeat(25)),
        "../../../../../etc/passwd/".to_owned(),
        "编号".repeat(13),
    ] {
        assert_eq!(bad.parse::<FileId>(), Err(InvalidFileId), "{bad}");
        assert!(serde_json::from_value::<FileId>(json!(bad)).is_err());
    }
    assert_eq!(serde_json::to_value(id()).unwrap(), json!(ID));
    assert_eq!(serde_json::from_value::<FileId>(json!(ID)).unwrap(), id());
}

#[test]
fn links_are_removed_from_the_body_with_their_markdown() {
    let link = link();
    let cases = [
        (link.clone(), ""),
        (format!("照片：{link}"), "照片："),
        (format!("{link}就是它"), "就是它"),
        (format!("照片是{link}，请查收。"), "照片是，请查收。"),
        (format!("给你 {link}."), "给你 ."),
        (format!("给你 {link} 了"), "给你 了"),
        (format!("{link} 给你"), "给你"),
        (format!("给你 [照片.jpg]({link}) 了"), "给你 了"),
        (format!("给你 ![照片]({link})"), "给你"),
        (format!("[照片]({link} \"七月\")"), ""),
        (format!("[{link}]({link})"), ""),
        (format!("<{link}>"), ""),
        (format!("`{link}`"), ""),
        (format!("**{link}**"), ""),
        (format!("（{link}）"), "（）"),
        (format!("第一段\n\n{link}\n\n第二段"), "第一段\n\n第二段"),
        (format!("第一行\n{link}\n第二行"), "第一行\n第二行"),
        (format!("上一行\n  缩进 {link}"), "上一行\n  缩进"),
    ];
    for (text, body) in cases {
        let found = extract_file_links(&text);
        assert_eq!(found.body, body, "{text}");
        assert_eq!(found.files, vec![id()], "{text}");
    }
}

#[test]
fn several_links_keep_their_order_without_duplicates() {
    let (first, second) = (id().link(), other_id().link());
    let found = extract_file_links(&format!(
        "两个文件：\n- {second}\n- [a]({first})\n1. {second}\n\n看看"
    ));
    assert_eq!(found.body, "两个文件：\n\n看看");
    assert_eq!(found.files, vec![other_id(), id()]);

    let found = extract_file_links(&format!("{first} {second}"));
    assert_eq!(found.body, "");
    assert_eq!(found.files, vec![id(), other_id()]);
}

/// 契约：一次发送里的文件合成一条消息（ADR-0094 第 2 节），收到的一方拆回原来的字和文件。
#[test]
fn a_files_message_carries_the_text_then_one_link_per_line() {
    let body = files_message_body("  这是上个月的账单\n", &[id(), other_id()]);
    assert_eq!(
        body,
        format!("这是上个月的账单\n\n{}\n{}", id().link(), other_id().link())
    );
    let found = extract_file_links(&body);
    assert_eq!(found.body.trim_end(), "这是上个月的账单");
    assert_eq!(found.files, vec![id(), other_id()]);

    assert_eq!(files_message_body(" ", &[id()]), id().link());
    assert_eq!(files_message_body("只有字", &[]), "只有字");
}

/// 不够严格时，别的 URL 或者模型改过的链接会被当成文件，从正文里吞掉。
#[test]
fn text_that_is_not_exactly_a_link_is_left_alone() {
    let short = &ID[..25];
    for text in [
        format!("wonton://file/{short}"),
        format!("wonton://file/{ID}a"),
        format!("wonton://file/{ID}/"),
        format!("wonton://file/{ID}/thumbnail"),
        format!("wonton://file/{ID}.jpg"),
        format!("wonton://file/{ID}?name=a"),
        format!("wonton://file/{ID}#a"),
        format!("WONTON://file/{ID}"),
        format!("wonton://FILE/{ID}"),
        format!("wonton://file/{}", ID.to_uppercase()),
        format!("xwonton://file/{ID}"),
        format!("https://example.com/wonton://file/{ID}"),
        format!("https://example.com/?u=wonton://file/{ID}"),
        "wonton://file/".to_owned(),
        "wonton://file/编号编号编号编号编号编号编号编号编".to_owned(),
        "wonton://pair?code=abc  多个  空格\n\n\n还有空行 ".to_owned(),
    ] {
        let found = extract_file_links(&text);
        assert_eq!(found.body, text);
        assert_eq!(found.files, vec![]);
    }
}

/// 契约：和 ADR-0020 的“错误”一节一一对应。类别写在 HTTP 响应、响应头和工具错误里，改了两边就对不上。
#[test]
fn failure_kinds_and_http_statuses_are_stable() {
    let table = [
        (FileFailure::Unauthenticated, "unauthenticated", Some(401)),
        (FileFailure::NotShared, "not_shared", Some(403)),
        (
            FileFailure::PermissionDenied,
            "permission_denied",
            Some(403),
        ),
        (FileFailure::FileUnknown, "file_unknown", Some(404)),
        (FileFailure::FileExpired, "file_expired", None),
        (FileFailure::FileGone, "file_gone", Some(410)),
        (FileFailure::FileChanged, "file_changed", Some(409)),
        (FileFailure::NotViewable, "not_viewable", Some(404)),
        (FileFailure::InvalidInput, "invalid_input", Some(400)),
        (FileFailure::StorageFull, "storage_full", Some(507)),
        (
            FileFailure::PluginUnavailable,
            "plugin_unavailable",
            Some(503),
        ),
        (FileFailure::UpstreamError, "upstream_error", Some(502)),
        (
            FileFailure::ResourceTooLarge,
            "resource_too_large",
            Some(502),
        ),
        (
            FileFailure::ServiceUnavailable,
            "service_unavailable",
            Some(503),
        ),
        (FileFailure::Internal, "internal", Some(500)),
        (
            FileFailure::TransportUnavailable,
            "transport_unavailable",
            None,
        ),
        (FileFailure::Protocol, "protocol", None),
        (FileFailure::UnsupportedFormat, "unsupported_format", None),
        (
            FileFailure::LocalSourceUnavailable,
            "local_source_unavailable",
            None,
        ),
        (
            FileFailure::LocalPermissionDenied,
            "local_permission_denied",
            None,
        ),
        (FileFailure::LocalStorageFull, "local_storage_full", None),
        (FileFailure::LocalIo, "local_io", None),
    ];
    for (failure, kind, status) in table {
        assert_eq!(failure.as_str(), kind);
        assert_eq!(serde_json::to_value(failure).unwrap(), json!(kind));
        assert_eq!(
            serde_json::from_value::<FileFailure>(json!(kind)).unwrap(),
            failure
        );
        assert_eq!(failure.http_status(), status, "{kind}");
    }
}

/// 契约：HTTP 错误体和请求头的写法。
#[test]
fn http_wire_format_is_stable() {
    let body = FileErrorBody {
        error: FileError {
            kind: FileFailure::FileGone,
            message: "文件已被移动或删除".into(),
        },
    };
    assert_eq!(
        serde_json::to_value(&body).unwrap(),
        json!({"error": {"kind": "file_gone", "message": "文件已被移动或删除"}})
    );
    assert_eq!(round_trip(&body), body);
    assert_eq!(FILES_HEADER, "X-Wonton-Files");
    assert_eq!(FILE_ERROR_HEADER, "X-Wonton-Error");
    assert_eq!(
        serde_json::to_value(FileGrantRequest {
            room: "!room:home.example".into(),
        })
        .unwrap(),
        json!({"room": "!room:home.example"})
    );
}

/// 契约：`GET /_wonton/files/{id}` 的写法。
#[test]
fn file_info_wire_format_is_stable() {
    let info = FileInfo {
        id: id(),
        name: "IMG_1234.jpg".into(),
        mime_type: Some("image/jpeg".into()),
        size: Some(4_404_019),
        source: FileSource::Bot {
            name: "hermes".into(),
            plugin: "app.immich".into(),
        },
        has_thumbnail: true,
    };
    assert_eq!(
        serde_json::to_value(&info).unwrap(),
        json!({
            "id": ID,
            "name": "IMG_1234.jpg",
            "mime_type": "image/jpeg",
            "size": 4_404_019,
            "source": {"type": "bot", "name": "hermes", "plugin": "app.immich"},
            "has_thumbnail": true,
        })
    );
    assert_eq!(round_trip(&info), info);
}

/// 核心可能比 App 新：不认识的取值落到兜底的变体，多出来的字段被忽略。
#[test]
fn values_from_a_newer_core_still_parse() {
    let info: FileInfo = serde_json::from_value(json!({
        "id": ID,
        "name": "a",
        "mime_type": null,
        "size": null,
        "source": {"type": "family", "member": "妈妈"},
        "has_thumbnail": false,
        "added_in_a_later_version": true,
    }))
    .unwrap();
    assert_eq!(info.source, FileSource::Unknown);
    let body: FileErrorBody = serde_json::from_value(json!({
        "error": {"kind": "quota_exceeded", "message": "x"},
    }))
    .unwrap();
    assert_eq!(body.error.kind, FileFailure::Other);
}

#[test]
fn cards_transfers_and_actions_round_trip_through_json() {
    let info = FileInfo {
        id: id(),
        name: "a.txt".into(),
        mime_type: None,
        size: None,
        source: FileSource::Device,
        has_thumbnail: false,
    };
    for card in [
        FileCard::Loading,
        FileCard::Ready { info },
        FileCard::Failed {
            reason: FileFailure::NotShared,
            message: "没有分享给你".into(),
        },
    ] {
        assert_eq!(round_trip(&card), card);
    }
    for state in [
        TransferState::Running,
        TransferState::Paused,
        TransferState::Done,
        TransferState::LinkNotSent {
            reason: FileFailure::ServiceUnavailable,
            message: "Matrix 服务器连不上".into(),
        },
        TransferState::Failed {
            reason: FileFailure::StorageFull,
            message: "磁盘放不下".into(),
        },
    ] {
        let transfer = Transfer {
            id: "local-1".into(),
            file: None,
            name: "a.mov".into(),
            direction: TransferDirection::Upload,
            room: Some("!room:home.example".into()),
            transferred_bytes: 1024,
            total_bytes: Some(4096),
            state,
            paused_because: Some("连不上 tusd".into()),
            destination: None,
            actions: vec![TransferAction::ResendLink, TransferAction::StopSending],
        };
        assert_eq!(round_trip(&transfer), transfer);
    }
    for tap in [
        FileCardAction::Play,
        FileCardAction::Download,
        FileCardAction::Open {
            path: "/data/downloads/a.mov".into(),
        },
        FileCardAction::Wait,
    ] {
        let actions = FileCardActions {
            tap: tap.clone(),
            open_elsewhere: Some(tap),
            download: None,
        };
        assert_eq!(round_trip(&actions), actions);
    }
    let view = TransfersViewModel {
        active: Vec::new(),
        downloaded: Vec::new(),
        uploaded: Vec::new(),
        problem: Some(FileProblem {
            operation: FileOperation::Delete,
            file: id(),
            reason: FileFailure::FileChanged,
            message: "已经是别的文件了".into(),
        }),
    };
    assert_eq!(round_trip(&view), view);
}

/// 契约：这些写进手机上的 `transfers.json`。改了写法，旧文件就读不懂，整个列表会被挪到 `.bad`。
#[test]
fn what_is_stored_on_the_phone_keeps_its_spelling() {
    let failed = |kind: &str| json!({"type": kind, "reason": "storage_full", "message": "满了"});
    let reason = || (FileFailure::StorageFull, "满了".to_owned());
    for (state, stored) in [
        (TransferState::Running, json!({"type": "running"})),
        (TransferState::Paused, json!({"type": "paused"})),
        (TransferState::Done, json!({"type": "done"})),
        (
            TransferState::LinkNotSent {
                reason: reason().0,
                message: reason().1,
            },
            failed("link_not_sent"),
        ),
        (
            TransferState::Failed {
                reason: reason().0,
                message: reason().1,
            },
            failed("failed"),
        ),
    ] {
        assert_eq!(serde_json::to_value(&state).unwrap(), stored);
        assert_eq!(
            serde_json::from_value::<TransferState>(stored).unwrap(),
            state
        );
    }
    for (source, stored) in [
        (
            UploadSource::Path { path: "/a".into() },
            json!({"type": "path", "path": "/a"}),
        ),
        (
            UploadSource::Opaque {
                handle: "content://a".into(),
            },
            json!({"type": "opaque", "handle": "content://a"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&source).unwrap(), stored);
        assert_eq!(
            serde_json::from_value::<UploadSource>(stored).unwrap(),
            source
        );
    }
}

#[test]
fn file_actions_round_trip_through_json() {
    for action in [
        Action::UploadFile {
            source: UploadSource::Opaque {
                handle: "content://media/external/images/1".into(),
            },
            name: "a.jpg".into(),
            mime_type: Some("image/jpeg".into()),
            room: Some("!room:home.example".into()),
        },
        Action::UploadFile {
            source: UploadSource::Path {
                path: "/tmp/a.mov".into(),
            },
            name: "a.mov".into(),
            mime_type: None,
            room: None,
        },
        Action::DownloadFile { id: id() },
        Action::CancelTransfer {
            id: "local-1".into(),
        },
        Action::DismissTransfer {
            id: "local-1".into(),
            delete_local_file: true,
        },
        Action::ResendFileLink {
            id: "local-1".into(),
        },
        Action::DeleteFile { id: id() },
        Action::SendFileToDevices { id: id() },
        Action::GrantFileToRoom {
            id: id(),
            room: "!room:home.example".into(),
        },
        Action::DismissFileProblem,
    ] {
        assert_eq!(round_trip(&action), action);
    }
}

fn upload_to(room: Option<&str>, state: TransferState) -> Transfer {
    Transfer {
        id: "local-1".into(),
        file: None,
        name: "a.mov".into(),
        direction: TransferDirection::Upload,
        room: room.map(str::to_owned),
        transferred_bytes: 0,
        total_bytes: None,
        state,
        paused_because: None,
        destination: None,
        actions: Vec::new(),
    }
}

fn room_row(room_id: &str, name: &str) -> RoomRow {
    RoomRow {
        layout: Default::default(),
        call_active: false,
        room_id: room_id.into(),
        name: name.into(),
        is_direct: true,
        direct_user_id: None,
        plugin: None,
        is_encrypted: true,
        is_read_only: false,
        membership: crate::RoomMembership::Joined,
        preview: None,
        side_chats: false,
        unread_count: 0,
    }
}

#[test]
fn a_room_lists_its_own_transfers_and_what_went_wrong_elsewhere() {
    let failed = TransferState::Failed {
        reason: FileFailure::StorageFull,
        message: "磁盘放不下".into(),
    };
    let not_sent = TransferState::LinkNotSent {
        reason: FileFailure::ServiceUnavailable,
        message: "发不出去".into(),
    };
    let view = TransfersViewModel {
        active: vec![
            upload_to(Some("!here"), TransferState::Running),
            upload_to(None, TransferState::Paused),
            upload_to(Some("!other"), TransferState::Running),
            upload_to(Some("!other"), TransferState::Paused),
            upload_to(Some("!other"), failed.clone()),
            upload_to(Some("!left"), not_sent.clone()),
        ],
        ..TransfersViewModel::default()
    };
    let listed = view.for_room(
        "!here",
        &[room_row("!other", "妈妈"), room_row("!here", "我")],
    );
    assert_eq!(
        listed.here,
        [
            upload_to(Some("!here"), TransferState::Running),
            upload_to(None, TransferState::Paused),
        ]
    );
    assert_eq!(
        listed.elsewhere,
        [
            TransferElsewhere {
                transfer: upload_to(Some("!other"), failed),
                room_name: Some("妈妈".into()),
            },
            TransferElsewhere {
                transfer: upload_to(Some("!left"), not_sent),
                room_name: None,
            },
        ]
    );
}
