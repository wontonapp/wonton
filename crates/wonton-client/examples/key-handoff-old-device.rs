//! 扮演加设备时的旧设备（ADR-0087）：真实的客户端核心，经隧道配对、登录 Matrix、开好备份，打开“添加设备”把配对码写进
//! 文件，等新设备来请求确认；对表情时打印出来并替主人点“相同”，交完密钥后再留一会儿，等新设备取走密钥。
//! 没有第二台 App 时用它和 App 做端到端验证，见 deploy/dev/core/README.md。
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
    HomeAction, KeyHandoff, Network, RecoveryStatus, SecretString, VerificationStage,
};

/// 从配对到新设备确认完。新设备那边要人去扫码、点按钮，给足时间。
const WAIT: Duration = Duration::from_secs(900);
/// 确认完以后新设备才来要密钥（经 to-device 消息），旧设备得在线回答。
const STAY: Duration = Duration::from_secs(120);

#[tokio::main]
async fn main() -> ExitCode {
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
    let view = wait_chat(&core, |view| {
        let encryption = &view.encryption;
        encryption.verification == DeviceVerification::Verified
            && encryption.recovery == RecoveryStatus::Enabled
    })
    .await?;
    if view.encryption.recovery_key.is_some() {
        core.dispatch(Action::Chat {
            action: ChatAction::ConfirmRecoveryKeySaved,
        })
        .map_err(|e| e.to_string())?;
    }
    println!("已登录、已验证、备份已开");

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
