#![recursion_limit = "256"]
//! 开发环境里以主人身份收发一个加密房间：`cargo run -p wonton-acp-bridge --example matrix-chat -- <房间 ID> <状态目录>`。
//! 从标准输入逐行发送，Bot 的新消息打印到标准输出。

use std::{path::PathBuf, sync::Arc};

use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use wonton_matrix::{
    ChatRoom, MatrixSession, MessageBody, OwnedRoomId, SessionConfig, StoreConfig,
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("ERROR: {error:?}");
        let mut source = error.source();
        while let Some(cause) = source {
            eprintln!("CAUSE: {cause:?}");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let room_id: OwnedRoomId = args.next().expect("需要房间 ID").parse()?;
    let store = PathBuf::from(args.next().expect("需要状态目录"));
    let homeserver = std::env::var("WONTON_MATRIX_HOMESERVER")?;
    let token = std::env::var("WONTON_MATRIX_LOGIN_TOKEN")?;
    let session = Arc::new(
        MatrixSession::login_with_jwt(
            &SessionConfig {
                homeserver_url: homeserver,
                device_name: "wonton acp c2 owner".into(),
                store: StoreConfig::Sqlite {
                    path: store,
                    passphrase: None,
                },
                proxy: None,
            },
            &token,
        )
        .await?,
    );
    session.start_sync().await?;
    let room = Arc::new(session.watch_room(&room_id).await?);
    let mut updates = room.message_updates().await;
    let initial = updates.next().await.unwrap_or_default();
    let mut seen = initial
        .iter()
        .filter_map(|message| message.event_id.as_ref().map(ToString::to_string))
        .collect::<std::collections::HashSet<_>>();
    eprintln!("就绪，输入文字发送");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                if !line.trim().is_empty() {
                    send(&room, line.trim()).await?;
                }
            }
            Some(messages) = updates.next() => {
                for message in messages {
                    let Some(event_id) = message.event_id.as_ref() else { continue };
                    if !seen.insert(event_id.to_string()) || message.is_own {
                        continue;
                    }
                    if let MessageBody::Text(text) = message.body {
                        println!("BOT: {text}");
                    }
                }
            }
        }
    }
    Ok(())
}

async fn send(room: &ChatRoom, text: &str) -> Result<(), wonton_matrix::Error> {
    room.send_text(text).await?;
    eprintln!("SENT");
    Ok(())
}
