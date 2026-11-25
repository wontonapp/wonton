//! 扮演加设备时的旧设备（ADR-0087）：真实的客户端核心，经隧道配对、登录 Matrix、开好备份，打开“添加设备”把配对码写进
//! 文件，等新设备来请求确认；对表情时打印出来并替主人点“相同”，交完密钥后再留一会儿，等新设备取走密钥。
//! 开备份时把恢复密钥写进 `<写配对码的文件>.recovery`，并先往默认 Bot 的私聊发一句加密历史（`KEY_HANDOFF_HISTORY_TEXT`），
//! 给“恢复密钥恢复”和“历史解密”两条走查留证据。没有第二台 App 时用它和 App 做端到端验证，见 deploy/dev/core/README.md。
//!
//! ```text
//! cargo run -p wonton-client --example key-handoff-old-device -- <数据目录> <配对码，已配对时写 -> <写配对码的文件>
//! ```

// 同 wonton-client：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use wonton_client::{
    Action, ChatAccount, ChatAction, ChatViewModel, ClientCore, DeviceInvite, DeviceVerification,
    HomeAction, KeyHandoff, MessageSummary, Network, RecoveryStatus, RoomMembership, SecretString,
    TimelineAction, TimelineTarget, VerificationStage,
};

/// 从配对到新设备确认完。新设备那边要人去扫码、点按钮，给足时间。
const WAIT: Duration = Duration::from_secs(1800);
/// 确认完以后新设备才来要密钥（经 to-device 消息），旧设备得在线回答。
const STAY: Duration = Duration::from_secs(120);

#[tokio::main]
async fn main() -> ExitCode {
    wonton_client::init_logging(std::env::var("RUST_LOG").unwrap_or_default(), None);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, code, out] = args.as_slice() else {
        eprintln!(
            "用法：key-handoff-old-device <数据目录> <配对码，已配对时写 -> <写配对码的文件>"
        );
        return ExitCode::from(2);
    };
    match tokio::time::timeout(WAIT, run(dir, code, out)).await {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("{WAIT:?} 内没有走完");
            ExitCode::FAILURE
        }
    }
}

async fn run(dir: &str, code: &str, out: &str) -> Result<(), String> {
    let core = ClientCore::with_network(dir, Network::DirectOnly).map_err(|e| e.to_string())?;
    if code != "-" {
        core.dispatch(Action::Home {
            action: HomeAction::PairHome {
                code: SecretString::new(code.to_owned()),
                device_name: "old-device".to_owned(),
            },
        })
        .map_err(|e| e.to_string())?;
    }
    enable_backup(&core, out).await?;
    leave_history(&core).await?;

    core.dispatch(Action::Home {
        action: HomeAction::InviteDevice,
    })
    .map_err(|e| e.to_string())?;
    let code = invite_code(&core).await?;
    std::fs::write(out, &code).map_err(|e| format!("写不了 {out}：{e}"))?;
    println!("“添加设备”的码已写进 {out}");

    let view = wait_chat(&core, |view| {
        let handoff = view.encryption.key_handoff;
        handoff == Some(KeyHandoff::CannotGive)
            || view
                .encryption
                .verification_flow
                .as_ref()
                .is_some_and(|flow| {
                    !flow.started_here && matches!(flow.stage, VerificationStage::Compare { .. })
                })
    })
    .await?;
    let Some(flow) = view
        .encryption
        .verification_flow
        .filter(|_| view.encryption.key_handoff == Some(KeyHandoff::Giving))
    else {
        return Err(format!("交不出密钥：{:?}", view.encryption.key_handoff));
    };
    if let VerificationStage::Compare { emoji, .. } = &flow.stage {
        let symbols: Vec<_> = emoji.iter().map(|e| e.symbol.as_str()).collect();
        println!("表情：{}", symbols.join(" "));
    }
    core.dispatch(Action::Chat {
        action: ChatAction::ConfirmVerification {
            flow_id: flow.flow_id.clone(),
        },
    })
    .map_err(|e| e.to_string())?;
    let view = wait_chat(&core, |view| {
        view.encryption
            .verification_flow
            .as_ref()
            .is_some_and(|flow| {
                matches!(
                    flow.stage,
                    VerificationStage::Done | VerificationStage::Cancelled { .. }
                )
            })
    })
    .await?;
    let stage = view.encryption.verification_flow.map(|flow| flow.stage);
    if stage != Some(VerificationStage::Done) {
        return Err(format!("确认没有完成：{stage:?}"));
    }
    println!(
        "确认完成，交接：{:?}；再留 {STAY:?} 回答新设备要密钥",
        view.encryption.key_handoff
    );
    tokio::time::sleep(STAY).await;
    core.dispatch(Action::Home {
        action: HomeAction::CloseDeviceInvite,
    })
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 等本设备被当成已验证、备份开起来，把恢复密钥抄到文件并确认已保存。
async fn enable_backup(core: &Arc<ClientCore>, out: &str) -> Result<(), String> {
    wait_chat(core, |view| {
        let encryption = &view.encryption;
        encryption.verification == DeviceVerification::Verified
            && encryption.recovery == RecoveryStatus::Enabled
    })
    .await?;
    // 开启备份和把密钥交到快照之间有间隔，等它出现再抄；走查要拿这把钥匙验“没有旧设备时用恢复密钥恢复”。
    let keyed = tokio::time::timeout(
        Duration::from_secs(30),
        wait_chat(core, |v| v.encryption.recovery_key.is_some()),
    )
    .await
    .ok()
    .and_then(Result::ok);
    if let Some(key) = keyed.and_then(|v| v.encryption.recovery_key) {
        let key_file = format!("{out}.recovery");
        std::fs::write(&key_file, key.expose()).map_err(|e| format!("写不了 {key_file}：{e}"))?;
        println!("恢复密钥已写进 {key_file}");
        core.dispatch(Action::Chat {
            action: ChatAction::ConfirmRecoveryKeySaved,
        })
        .map_err(|e| e.to_string())?;
    }
    println!("已登录、已验证、备份已开");
    Ok(())
}

/// 留在备份里的加密历史：新设备拿到密钥后进这个私聊的时间线应该读到这句明文。
/// 发出去只说明派发成功，等它真的出现在房间预览里再往下走（示例退出、没写码时，因果就在这一行）。
async fn leave_history(core: &Arc<ClientCore>) -> Result<(), String> {
    let history = std::env::var("KEY_HANDOFF_HISTORY_TEXT").unwrap_or_else(|_| {
        format!(
            "历史消息 {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        )
    });
    // 先问 Bot。旧会话里恢复不了的设备确认请求会让这一步当场报错、也不发消息：等不到就走直发。
    let asked = core
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: history.clone(),
            },
        })
        .is_ok();
    if !asked
        || wait_preview(core, &history, Duration::from_secs(30))
            .await
            .is_err()
    {
        send_history_direct(core, &history).await?;
        wait_preview(core, &history, Duration::from_secs(60)).await?;
    }
    println!("历史消息已发出：{history}");
    Ok(())
}

/// 等这句以自己消息的形式出现在任一房间预览里。
async fn wait_preview(core: &Arc<ClientCore>, history: &str, wait: Duration) -> Result<(), String> {
    tokio::time::timeout(
        wait,
        wait_chat(core, |view| {
            view.rooms.iter().any(|room| {
                room.preview.as_ref().is_some_and(|preview| {
                    preview.is_own
                        && matches!(&preview.body, MessageSummary::Text { text } if text == history)
                })
            })
        }),
    )
    .await
    .map_err(|_| format!("{wait:?} 内历史消息没进房间：{history}"))?
    .map(|_| ())
}

/// 直接建和 Bot 的私聊、订阅时间线再发；问 Bot 没就绪或报错时走这里。
async fn send_history_direct(core: &Arc<ClientCore>, body: &str) -> Result<(), String> {
    let bot = std::env::var("KEY_HANDOFF_BOT_USER")
        .map_err(|_| "问 Bot 没就绪，也没给 KEY_HANDOFF_BOT_USER".to_owned())?;
    core.dispatch(Action::Chat {
        action: ChatAction::StartDirectChat {
            user_id: bot.clone(),
        },
    })
    .map_err(|e| e.to_string())?;
    let view = wait_chat(core, |view| {
        view.rooms.iter().any(|room| {
            room.direct_user_id.as_deref() == Some(bot.as_str())
                && room.membership == RoomMembership::Joined
        })
    })
    .await?;
    let room = view
        .rooms
        .iter()
        .find(|room| room.direct_user_id.as_deref() == Some(bot.as_str()))
        .map(|room| room.room_id.clone())
        .ok_or_else(|| "建不出和 Bot 的私聊".to_owned())?;
    let target = TimelineTarget {
        room_id: room,
        thread_root: None,
    };
    let _subscription = core
        .subscribe_timeline(target.clone())
        .await
        .map_err(|e| e.to_string())?;
    let mut last = None;
    for _ in 0..10 {
        let sent = core.dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: target.clone(),
                body: body.to_owned(),
                reply_to: None,
                mentions: Vec::new(),
            },
        });
        match sent {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error.to_string());
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    Err(last.unwrap_or_default())
}

async fn invite_code(core: &Arc<ClientCore>) -> Result<String, String> {
    let updates = core.subscribe_home();
    while let Some(view) = updates.next().await {
        match view.invite {
            Some(DeviceInvite::Ready { code, .. }) => return Ok(code.expose().to_owned()),
            Some(DeviceInvite::Failed { message, .. }) => {
                return Err(format!("“添加设备”要不到码：{message}"));
            }
            _ => {}
        }
    }
    Err("连接的订阅提前结束".to_owned())
}

async fn wait_chat(
    core: &Arc<ClientCore>,
    done: impl Fn(&ChatViewModel) -> bool,
) -> Result<ChatViewModel, String> {
    let updates = core.subscribe_chat();
    while let Some(view) = updates.next().await {
        if std::env::var_os("KEY_HANDOFF_DEBUG").is_some() {
            eprintln!(
                "view: verif={:?} rec={:?} key={} flow={:?}",
                view.encryption.verification,
                view.encryption.recovery,
                view.encryption.recovery_key.is_some(),
                view.encryption
                    .verification_flow
                    .as_ref()
                    .map(|f| f.stage.clone()),
            );
        }
        if let ChatAccount::Failed { message, .. } = &view.account {
            return Err(format!("登录 Matrix 失败：{message}"));
        }
        if let Some(problem) = &view.problem {
            return Err(format!("聊天出错：{problem:?}"));
        }
        if done(&view) {
            return Ok(view);
        }
    }
    Err("聊天的订阅提前结束".to_owned())
}
