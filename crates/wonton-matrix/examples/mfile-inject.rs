//! m.file 本地注入工具：以真实 matrix-sdk 客户端（含 Megolm）登录开发栈的 Tuwunel，向指定账号发真实 m.file。
//! App 自身不发 m.file 事件；要验收到端的文件卡片，这是收到端之外唯一能产生真实加密 m.file 的路子。
//!
//! ```bash
//! WONTON_INJECT_KEY=<栈的 matrix-jwt.pem> \
//!   cargo run -p wonton-matrix --features testkit --example mfile-inject -- \
//!   <tuwunel 直连地址> @owner:wonton.localhost [说明文字]
//! ```
//!
//! homeserver_url 钉死直连地址，不走 .well-known：Tuwunel 的 well-known 指回核心网关的 Matrix 代理，
//! 那里只认经隧道接入的已配对设备，注入器过不去。
//!
//! 建「加密」「明文」两个房间邀请对方，各发一个 PDF（加密间带说明文字）；等对方加入最多 10 分钟。
#![recursion_limit = "256"]

use std::time::Duration;

use matrix_sdk::{
    Client, RoomState,
    attachment::{AttachmentConfig, AttachmentInfo, BaseFileInfo},
    config::SyncSettings,
    ruma::{
        OwnedRoomId, OwnedUserId,
        api::client::room::create_room::{self, v3::RoomPreset},
        events::room::{encryption::RoomEncryptionEventContent, message::TextMessageEventContent},
        serde::Raw,
    },
};

/// Tuwunel 的 JWT 登录类型（同 wonton-matrix/src/session.rs）。
const JWT_LOGIN_TYPE: &str = "org.matrix.login.jwt";

/// 一页最小的合法 PDF：一页一行字，无压缩内容流。
fn sample_pdf() -> Vec<u8> {
    let stream = b"BT /F1 18 Tf 72 700 Td (2025 Tax Return) Tj ET";
    let objects: Vec<Vec<u8>> = vec![
        b"1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n".to_vec(),
        b"2 0 obj\n<</Type/Pages/Kids[3 0 R]/Count 1>>\nendobj\n".to_vec(),
        b"3 0 obj\n<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents 4 0 R\
/Resources<</Font<</F1 5 0 R>>>>>>\nendobj\n"
            .to_vec(),
        format!(
            "4 0 obj\n<</Length {}>>\nstream\n{}\nendstream\nendobj\n",
            stream.len(),
            String::from_utf8_lossy(stream)
        )
        .into_bytes(),
        b"5 0 obj\n<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>\nendobj\n".to_vec(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for object in &objects {
        offsets.push(out.len());
        out.extend_from_slice(object);
    }
    let xref = out.len();
    out.extend_from_slice(
        &format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).into_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(&format!("{offset:010} 00000 n \n").into_bytes());
    }
    out.extend_from_slice(
        &format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .into_bytes(),
    );
    out
}

async fn new_room(client: &Client, invite: &OwnedUserId, encrypted: bool) -> OwnedRoomId {
    let mut request = create_room::v3::Request::new();
    request.invite = vec![invite.clone()];
    request.is_direct = true;
    request.preset = Some(RoomPreset::PrivateChat);
    request.name = Some(if encrypted {
        "文件冒烟-加密".into()
    } else {
        "文件冒烟-明文".into()
    });
    if encrypted {
        // initial_state 要完整的事件形状：type + content（typed content 的序列化只是 content 本身）。
        let content = serde_json::to_value(RoomEncryptionEventContent::with_recommended_defaults())
            .expect("加密事件能序列化");
        let event = serde_json::json!({ "type": "m.room.encryption", "content": content });
        request.initial_state = vec![Raw::new(&event).expect("裸 JSON").cast_unchecked()];
    }
    client
        .create_room(request)
        .await
        .expect("建房间")
        .room_id()
        .to_owned()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let homeserver = args.next().expect("tuwunel 直连地址");
    let invite: OwnedUserId = args.next().expect("对方 MXID").parse().expect("MXID");
    let caption = args.next().unwrap_or_else(|| "上个月的账单".into());

    let key_path = std::env::var("WONTON_INJECT_KEY")
        .unwrap_or_else(|_| "deploy/dev/matrix/.secrets/jwt-ed25519.pem".into());
    let pem = std::fs::read(key_path).expect("读签发私钥");
    let key = jsonwebtoken::EncodingKey::from_ed_pem(&pem).expect("读取 Ed25519 私钥（PEM）");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = serde_json::json!({
        "sub": "fsfriend",
        "iss": "wonton-core",
        "aud": "tuwunel",
        "iat": now,
        "exp": now + 120,
    });
    let jwt = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA),
        &claims,
        &key,
    )
    .expect("签发 JWT");

    let client = Client::builder()
        .homeserver_url(&homeserver)
        // Tuwunel 的登录响应带 well_known 指回核心网关的 Matrix 代理，那里只认隧道设备；不跟。
        .respect_login_well_known(false)
        .build()
        .await
        .expect("客户端");
    client
        .matrix_auth()
        .login_custom(
            JWT_LOGIN_TYPE,
            serde_json::json!({ "token": jwt })
                .as_object()
                .cloned()
                .expect("登录数据是对象"),
        )
        .expect("登录请求构造")
        .initial_device_display_name("注入器")
        .send()
        .await
        .expect("fsfriend 登录");
    println!(
        "INJECT as {} @ {homeserver}",
        client.whoami().await.expect("whoami").user_id
    );

    client
        .sync_once(SyncSettings::default().timeout(Duration::from_secs(10)))
        .await
        .expect("同步");
    // 自己的设备集合先就绪，房间密钥才分得出去。
    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;

    let encrypted_room = new_room(&client, &invite, true).await;
    println!("ENCRYPTED_ROOM={encrypted_room}");
    let plain_room = new_room(&client, &invite, false).await;
    println!("PLAIN_ROOM={plain_room}");

    for (room_id, with_caption) in [(&encrypted_room, true), (&plain_room, false)] {
        let room = client.get_room(room_id).expect("房间在列表里");
        let mut waited = Duration::ZERO;
        while room.state() != RoomState::Joined {
            if room.state() == RoomState::Invited {
                room.join().await.expect("fsfriend 进自己的房间");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            waited += Duration::from_secs(2);
            assert!(
                waited < Duration::from_secs(600),
                "等对方加入超时：{room_id}"
            );
        }
        println!("JOINED {room_id}");
        let pdf = sample_pdf();
        let mut config = AttachmentConfig::new().info(AttachmentInfo::File(BaseFileInfo {
            size: Some((pdf.len() as u64).try_into().unwrap()),
        }));
        if with_caption {
            // 等对方设备和房间密钥就位，Megolm 会话才带着对方。
            tokio::time::sleep(Duration::from_secs(5)).await;
            config = config.caption(Some(TextMessageEventContent::plain(caption.clone())));
        }
        room.send_queue()
            .send_attachment(
                "2025-报税表.pdf",
                "application/pdf".parse().unwrap(),
                pdf,
                config,
            )
            .await
            .expect("发附件");
        println!("SENT {room_id}");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    println!("INJECT_DONE");
}
