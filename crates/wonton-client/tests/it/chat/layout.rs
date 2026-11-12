//! 聊天显示方式（ADR-0060）：插件默认、主人的选择、全局默认和另一台设备之间的同步。

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use wonton_client::{
    Action, ChatFailure, ChatForm, ChatLayout, ChatLayoutChoice, ChatLayoutTarget, ChatSync,
    ChatTextScale, CodeOverflow, RoomMembership,
};
use wonton_matrix::{MatrixSession, OwnedUserId};
use wonton_protocol::PluginChat;

use super::support::{Home, OWNER, Phone, WAIT};

const DOCUMENT: &str = "app.writer";
const PLAIN: &str = "app.chatty";

async fn wait_layout(
    phone: &Phone,
    room_id: &str,
    what: &str,
    check: impl Fn(&ChatLayout) -> bool,
) -> ChatLayout {
    phone
        .wait_chat(what, |view| {
            view.rooms
                .iter()
                .find(|row| row.room_id == room_id)
                .map(|row| row.layout.clone())
                .filter(|layout| check(layout))
        })
        .await
}

/// 另一台设备上的账号数据，等它同步过来。
async fn wait_account_data(
    other: &MatrixSession,
    what: &str,
    check: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let data = other
            .chat_layout()
            .await
            .ok()
            .flatten()
            .map(|content| serde_json::to_value(content).unwrap());
        if let Some(data) = data.filter(|data| check(data)) {
            return data;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "账号数据没有同步过来：{what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn invite(phone: &Phone, bot: &MatrixSession, name: &str) -> String {
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room = bot.create_direct_chat(&owner, false).await.unwrap();
    phone
        .wait_chat("收到 Bot 的邀请", |view| {
            view.rooms
                .iter()
                .find(|row| row.direct_user_id.as_deref() == Some(bot.user_id().as_str()))
                .filter(|row| {
                    row.room_id == room.as_str() && row.membership == RoomMembership::Invited
                })
                .cloned()
        })
        .await;
    phone
        .dispatch(Action::JoinRoom {
            room_id: room.to_string(),
        })
        .unwrap();
    phone
        .wait_chat("加入房间", |view| {
            view.rooms
                .iter()
                .find(|row| row.room_id == room.as_str() && row.name == name)
                .filter(|row| row.membership == RoomMembership::Joined)
                .map(|row| row.layout.clone())
        })
        .await;
    room.to_string()
}

/// 已装列表：一个声明文档的 Bot，一个没声明（家里服务器给气泡）的 Bot。
fn plugins(writer: &MatrixSession, chatty: &MatrixSession) -> Vec<wonton_client::InstalledPlugin> {
    let mut declared = crate::plugins::rig::running(DOCUMENT);
    declared.title = "Writer".to_owned();
    declared.bot_user_id = Some(writer.user_id().to_string());
    declared.chat = Some(PluginChat {
        form: ChatForm::Document,
    });
    let mut undeclared = crate::plugins::rig::running(PLAIN);
    undeclared.title = "Chatty".to_owned();
    undeclared.bot_user_id = Some(chatty.user_id().to_string());
    undeclared.chat = Some(PluginChat {
        form: ChatForm::Bubbles,
    });
    vec![declared, undeclared]
}

fn bot(phone: &Phone, user: &MatrixSession) -> wonton_client::BotView {
    phone
        .chat()
        .bots
        .iter()
        .find(|bot| bot.user_id == user.user_id().as_str())
        .cloned()
        .expect("Bot 在列表里")
}

fn set(phone: &Phone, target: ChatLayoutTarget, choice: ChatLayoutChoice) {
    phone
        .dispatch(Action::SetChatLayout { target, choice })
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn chat_layouts_follow_the_plugin_the_owners_choice_and_the_global_default() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let second_dir = tempfile::tempdir().unwrap();
    let second = Phone::open(second_dir.path());
    let home = Home::start(&[phone.device_id(), second.device_id()]).await;
    let writer = Arc::new(home.bot("writer", "Writer").await);
    let chatty = Arc::new(home.bot("chatty", "Chatty").await);
    home.set_installed_plugins(plugins(&writer, &chatty));
    // 另一台设备：同一个主人账号，自己连 Tuwunel，用来读原始账号数据。
    let other = home.matrix.login("owner", "Other device").await;
    other.start_sync().await.unwrap();

    phone.connect(&home);
    phone.signed_in().await;
    let writer_room = invite(&phone, &writer, "Writer").await;
    let chatty_room = invite(&phone, &chatty, "Chatty").await;
    defaults(&phone, &writer, &writer_room, &chatty_room).await;
    choice(&phone, &writer, &writer_room, &other).await;
    other_device(&home, &second, &writer_room).await;
    global_default(&phone, &writer, &writer_room, &chatty_room, &second).await;
    unknown_value(&phone, &other, &chatty, &chatty_room).await;
    keeps_other_items(&phone, &other, &chatty, &chatty_room).await;
    write_failure(&home, &phone, &second, &chatty, &chatty_room).await;
}

/// 插件声明的形态和设备上的默认。
async fn defaults(phone: &Phone, writer: &MatrixSession, writer_room: &str, chatty_room: &str) {
    wait_layout(phone, writer_room, "插件声明的文档形态", |layout| {
        layout.form == ChatForm::Document
    })
    .await;
    wait_layout(phone, chatty_room, "没声明的气泡", |layout| {
        layout.form == ChatForm::Bubbles
    })
    .await;
    let writer = bot(phone, writer);
    assert_eq!(writer.chat.default.form, Some(ChatForm::Document));
    assert_eq!(writer.chat.choice, ChatLayoutChoice::default());
}

/// 主人把这个 Bot 改成气泡：房间跟着变，账号数据里只有这一项。
async fn choice(phone: &Phone, writer: &MatrixSession, writer_room: &str, other: &MatrixSession) {
    set(
        phone,
        ChatLayoutTarget::Bot {
            user_id: writer.user_id().to_string(),
        },
        ChatLayoutChoice {
            form: Some(ChatForm::Bubbles),
            ..ChatLayoutChoice::default()
        },
    );
    wait_layout(phone, writer_room, "改成气泡", |layout| {
        layout.form == ChatForm::Bubbles
    })
    .await;
    let expected = json!({"bots": {writer.user_id().to_string(): {"form": "bubbles"}}});
    let data = wait_account_data(other, "账号数据里只有这一项", |data| {
        data == &expected
    })
    .await;
    assert_eq!(data, expected);
}

/// 另一台设备登录同一账号，读到同样的结果。
async fn other_device(home: &Home, second: &Phone, writer_room: &str) {
    second.connect(home);
    second.signed_in().await;
    wait_layout(
        second,
        writer_room,
        "另一台设备也是气泡",
        |layout| layout.form == ChatForm::Bubbles,
    )
    .await;
}

/// 改全局默认：没被单独设过的 Bot 跟着变，设过的不变，插件默认也不变。
async fn global_default(
    phone: &Phone,
    writer: &MatrixSession,
    writer_room: &str,
    chatty_room: &str,
    second: &Phone,
) {
    set(
        second,
        ChatLayoutTarget::Default,
        ChatLayoutChoice {
            form: Some(ChatForm::Document),
            text_scale: Some(ChatTextScale::Larger),
            ..ChatLayoutChoice::default()
        },
    );
    let chatty_layout = wait_layout(phone, chatty_room, "跟着全局默认", |layout| {
        layout.form == ChatForm::Bubbles && layout.text_scale == ChatTextScale::Larger
    })
    .await;
    assert_eq!(chatty_layout.code, CodeOverflow::Scroll);
    let writer_layout = wait_layout(phone, writer_room, "设过的只跟没设的项", |layout| {
        layout.text_scale == ChatTextScale::Larger
    })
    .await;
    assert_eq!(writer_layout.form, ChatForm::Bubbles);
    let writer = bot(phone, writer);
    assert_eq!(writer.chat.default.form, Some(ChatForm::Document));
    assert_eq!(writer.chat.choice.form, Some(ChatForm::Bubbles));
}

/// 账号数据里有这个 App 不认识的取值：显示落到默认。
async fn unknown_value(
    phone: &Phone,
    other: &MatrixSession,
    chatty: &MatrixSession,
    chatty_room: &str,
) {
    other
        .update_chat_layout(|content| {
            content.insert(
                "bots".to_owned(),
                json!({chatty.user_id().to_string(): {"form": "hologram"}}),
            );
        })
        .await
        .unwrap();
    // 认不出的取值到了资料页，才算这次同步到了手机上。
    let choice = phone
        .wait_chat("认不出的取值进了资料页", |view| {
            view.bots
                .iter()
                .find(|bot| bot.user_id == chatty.user_id().as_str())
                .filter(|bot| bot.chat.choice.form == Some(ChatForm::Other))
                .map(|bot| bot.chat.clone())
        })
        .await;
    assert_eq!(choice.choice.form, Some(ChatForm::Other));
    let layout = wait_layout(
        phone,
        chatty_room,
        "认不出的取值落到默认",
        |layout| layout.form == ChatForm::Bubbles,
    )
    .await;
    assert_eq!(layout.text_scale, ChatTextScale::Larger);
}

/// 改别的项时认不出的取值原样保留。
async fn keeps_other_items(
    phone: &Phone,
    other: &MatrixSession,
    chatty: &MatrixSession,
    chatty_room: &str,
) {
    let mut choice = bot(phone, chatty).chat.choice;
    choice.code = Some(CodeOverflow::Wrap);
    set(
        phone,
        ChatLayoutTarget::Bot {
            user_id: chatty.user_id().to_string(),
        },
        choice,
    );
    let expected = json!({
        "default": {"form": "document", "text_scale": "larger"},
        "bots": {chatty.user_id().to_string(): {"form": "hologram", "code": "wrap"}}
    });
    wait_account_data(
        other,
        "改同一 Bot 的另一项也保留未知值",
        |data| data == &expected,
    )
    .await;

    let expected_default = ChatLayoutChoice {
        code: Some(CodeOverflow::Wrap),
        ..ChatLayoutChoice::default()
    };
    set(phone, ChatLayoutTarget::Default, expected_default.clone());
    phone
        .wait_chat("全局选择和房间显示均已生效", |view| {
            (view.chat_layout_default == expected_default
                && view.rooms.iter().any(|room| {
                    room.room_id == chatty_room
                        && room.layout.text_scale == ChatTextScale::Standard
                        && room.layout.code == CodeOverflow::Wrap
                }))
            .then_some(())
        })
        .await;
    let expected = json!({
        "default": {"code": "wrap"},
        "bots": {chatty.user_id().to_string(): {"form": "hologram", "code": "wrap"}}
    });
    let data = wait_account_data(other, "认不出的取值还在", |data| data == &expected).await;
    assert_eq!(data, expected);
}

/// 写不了时说明原因，显示不变。
///
/// 手机上可能还有更早的同步内容在路上。手机上自己写的响应是当场应用的，走在前头很正常；这时如果直接断言，
/// 落后的那份旧内容随后落下来，会把显示盖回旧值。先让另一台设备改一处别的值、等它从同步到手机上：这条只能
/// 经同步来，等到了就说明手机已经处理到写之后的状态，后面才不会被旧内容抢先。
async fn write_failure(
    home: &Home,
    phone: &Phone,
    other: &Phone,
    chatty: &MatrixSession,
    chatty_room: &str,
) {
    let chatty_id = chatty.user_id().to_string();
    set(
        other,
        ChatLayoutTarget::Bot { user_id: chatty_id },
        ChatLayoutChoice {
            code: Some(CodeOverflow::Scroll),
            ..ChatLayoutChoice::default()
        },
    );
    wait_layout(
        phone,
        chatty_room,
        "标记值从同步到了手机",
        |layout| layout.code == CodeOverflow::Scroll,
    )
    .await;
    // 写之前不能已经挂着别的原因，否则等一下会把旧原因当成这次的。
    phone
        .wait_chat("写之前没有别的原因", |view| {
            view.problem.is_none().then_some(())
        })
        .await;

    // Tuwunel 像挂了一样：隧道还在、会话还在，请求断在转发里。
    home.set_matrix_down(true);
    set(
        phone,
        ChatLayoutTarget::Default,
        ChatLayoutChoice {
            form: Some(ChatForm::Bubbles),
            ..ChatLayoutChoice::default()
        },
    );
    let problem = phone
        .wait_chat("写失败的原因", |view| view.problem.clone())
        .await;
    assert_eq!(problem.reason, ChatFailure::TransportUnavailable);
    let after = phone.chat();
    let default = &after.chat_layout_default;
    assert_eq!(default.form, None);
    assert_eq!(default.code, Some(CodeOverflow::Wrap));
    let chatty_layout = after
        .rooms
        .iter()
        .find(|room| room.room_id == chatty_room)
        .map(|room| room.layout.clone())
        .expect("Chatty 的房间还在");
    assert_eq!(
        chatty_layout.code,
        CodeOverflow::Scroll,
        "标记值也没被盖回去"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn saved_bot_choices_do_not_override_a_non_plugin_direct_chat() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.set_installed_plugins(Vec::new());
    let visitor = home.bot("visitor", "Visitor").await;
    let other = home.matrix.login("owner", "Other device").await;
    other
        .update_chat_layout(|raw| {
            raw.insert(
                "default".to_owned(),
                json!({"form": "document", "text_scale": "larger", "code": "wrap"}),
            );
            raw.insert(
                "bots".to_owned(),
                json!({visitor.user_id().to_string(): {
                    "form": "bubbles", "text_scale": "smaller", "code": "scroll"
                }}),
            );
        })
        .await
        .unwrap();
    phone.connect(&home);
    phone.signed_in().await;
    let room = invite(&phone, &visitor, "Visitor").await;
    let layout = wait_layout(
        &phone,
        &room,
        "非插件私聊采用全局显示方式",
        |layout| layout.form == ChatForm::Document,
    )
    .await;
    assert_eq!(
        layout,
        ChatLayout {
            form: ChatForm::Document,
            text_scale: ChatTextScale::Larger,
            code: CodeOverflow::Wrap,
        }
    );
}

/// 初次取账号数据失败要说出来；连接断过又回来后重新跟，账号数据的变化又能生效。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_failed_layout_read_is_reported_and_followed_again_after_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let writer = Arc::new(home.bot("writer", "Writer").await);
    let chatty = Arc::new(home.bot("chatty", "Chatty").await);
    home.set_installed_plugins(plugins(&writer, &chatty));
    // 手机上第一件事就是取账号数据：这时让取显示方式的那条请求断掉，别的照常。
    home.set_layout_down(true);
    phone.connect(&home);
    phone.signed_in().await;
    let writer_room = invite(&phone, &writer, "Writer").await;

    let problem = phone
        .wait_chat("取不到显示方式要说出来", |view| {
            view.problem.clone()
        })
        .await;
    assert_eq!(problem.reason, ChatFailure::TransportUnavailable);
    // 取不到时显示照旧：还是插件声明的文档形态（插件列表可能还没到，等它到）。
    let layout = wait_layout(
        &phone,
        &writer_room,
        "取不到也按插件默认",
        |layout| layout.form == ChatForm::Document,
    )
    .await;
    assert_eq!(layout.form, ChatForm::Document);

    // 连接断过又回来：同步停下、再跑起来以后重新跟。
    home.set_layout_down(false);
    home.set_matrix_down(true);
    phone
        .wait_chat("同步因断线停下", |view| {
            matches!(view.sync, ChatSync::Failed { .. }).then_some(())
        })
        .await;
    home.set_matrix_down(false);
    phone
        .wait_chat("跟上了以后问题收掉", |view| {
            view.problem.is_none().then_some(())
        })
        .await;

    let other = home.matrix.login("owner", "Other device").await;
    other
        .update_chat_layout(|raw| {
            raw.insert("default".to_owned(), json!({"text_scale": "larger"}));
        })
        .await
        .unwrap();
    let layout = wait_layout(
        &phone,
        &writer_room,
        "连接回来后账号数据又能反映到房间",
        |layout| layout.text_scale == ChatTextScale::Larger,
    )
    .await;
    assert_eq!(layout.form, ChatForm::Document, "插件默认还在");
}
