//! 扮演手机回答确认卡片（ADR-0016）：真实的客户端核心，经隧道配对、登录 Matrix，在“Wonton”私聊里等一张等回答的卡片，
//! 按命令行点按钮，等到最后的结果。没有 App 的主机上用它做端到端验证，见 deploy/dev/core/README.md。
//!
//! ```text
//! cargo run -p wonton-client --example confirm-card -- <数据目录> <配对码，已配对时写 -> <approve|reject>
//! ```

// 同 wonton-client：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use wonton_client::{
    Action, Answer, ChatAccount, ChatSync, ClientCore, ConfirmationState, ConfirmationView,
    MessageContent, Network, SecretString, TimelineContent, TimelineDiff, TimelineItem,
    TimelineSubscription,
};

/// 从开始等卡片到执行完。官方索引里的插件要拉镜像，给足时间。
const WAIT: Duration = Duration::from_secs(900);

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, code, answer] = args.as_slice() else {
        eprintln!("用法：confirm-card <数据目录> <配对码，已配对时写 -> <approve|reject>");
        return ExitCode::from(2);
    };
    let answer = match answer.as_str() {
        "approve" => Answer::Approve,
        "reject" => Answer::Reject,
        other => {
            eprintln!("不认识的回答：{other}");
            return ExitCode::from(2);
        }
    };
    match tokio::time::timeout(WAIT, run(dir, code, answer)).await {
        Ok(Ok(state)) => {
            println!("结果：{state:?}");
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("{WAIT:?} 内没有等到结果");
            ExitCode::FAILURE
        }
    }
}

async fn run(dir: &str, code: &str, answer: Answer) -> Result<ConfirmationState, String> {
    let core = ClientCore::with_network(dir, Network::DirectOnly).map_err(|e| e.to_string())?;
    if code != "-" {
        core.dispatch(Action::PairHome {
            code: SecretString::new(code.to_owned()),
            device_name: "confirm-card".to_owned(),
        })
        .map_err(|e| e.to_string())?;
    }
    let room_id = wonton_room(&core).await?;
    println!("Wonton 私聊：{room_id}");
    let timeline = core
        .subscribe_timeline(room_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    let view = next_view(&timeline, &mut items, |view| {
        view.card.state == ConfirmationState::Pending && !view.card.choices.is_empty()
    })
    .await?;
    println!("卡片：{}", view.card.title);
    for line in &view.card.lines {
        println!("  {}{}", if line.emphasis { "! " } else { "" }, line.text);
    }
    let labels: Vec<_> = view.card.choices.iter().map(|c| c.label.as_str()).collect();
    println!("按钮：{}", labels.join(" / "));
    let id = view.card.id;
    core.dispatch(Action::AnswerConfirmation {
        id: id.clone(),
        answer,
        choice_id: None,
    })
    .map_err(|e| e.to_string())?;
    let view = next_view(&timeline, &mut items, |view| {
        view.card.id == id
            && (view.problem.is_some()
                || !matches!(
                    view.card.state,
                    ConfirmationState::Pending | ConfirmationState::Running
                ))
    })
    .await?;
    match view.problem {
        Some(problem) => Err(format!("回答没有成功：{problem:?}")),
        None => Ok(view.card.state),
    }
}

/// 等登录，再等核心建好和主人的私聊（发第一张卡片时才建）。
async fn wonton_room(core: &Arc<ClientCore>) -> Result<String, String> {
    let updates = core.subscribe_chat();
    while let Some(view) = updates.next().await {
        if let ChatAccount::Failed { message, .. } = &view.account {
            return Err(format!("登录 Matrix 失败：{message}"));
        }
        let room = view
            .rooms
            .iter()
            .find(|room| {
                room.direct_user_id
                    .as_deref()
                    .is_some_and(|user| user.starts_with("@wonton-core:"))
            })
            .filter(|_| matches!(view.account, ChatAccount::SignedIn { .. }))
            .filter(|_| view.sync == ChatSync::Running);
        if let Some(room) = room {
            return Ok(room.room_id.clone());
        }
    }
    Err("聊天的订阅提前结束".to_owned())
}

async fn next_view(
    timeline: &Arc<TimelineSubscription>,
    items: &mut Vec<TimelineItem>,
    mut check: impl FnMut(&ConfirmationView) -> bool,
) -> Result<ConfirmationView, String> {
    loop {
        // 最新的卡片在后面。
        let found = items.iter().rev().find_map(|item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Confirmation { view } if check(view) => Some(view.clone()),
                _ => None,
            },
            _ => None,
        });
        if let Some(view) = found {
            return Ok(view);
        }
        let update = timeline
            .next()
            .await
            .ok_or("时间线的订阅提前结束".to_owned())?;
        for diff in update.diffs {
            apply(items, diff);
        }
    }
}

fn apply(items: &mut Vec<TimelineItem>, diff: TimelineDiff) {
    match diff {
        TimelineDiff::Append { items: more } => items.extend(more),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::PushFront { item } => items.insert(0, item),
        TimelineDiff::PushBack { item } => items.push(item),
        TimelineDiff::PopFront => {
            items.remove(0);
        }
        TimelineDiff::PopBack => {
            items.pop();
        }
        TimelineDiff::Insert { index, item } => items.insert(index as usize, item),
        TimelineDiff::Set { index, item } => items[index as usize] = item,
        TimelineDiff::Remove { index } => {
            items.remove(index as usize);
        }
        TimelineDiff::Truncate { length } => items.truncate(length as usize),
        TimelineDiff::Reset { items: all } => *items = all,
    }
}
