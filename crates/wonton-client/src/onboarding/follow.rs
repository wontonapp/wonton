//! 跟着家里的连接、聊天列表和发出的消息更新引导：换服务器时清掉记下的选择，连上时取状态，给默认 Agent 发过消息后
//! 不再显示候选提问。

use std::sync::{Arc, Weak};

use tokio::runtime::Handle;
use tokio::sync::watch::Receiver;
use wonton_protocol::HomeStatus;

use super::{OnboardingLink, lock, storage_problem};
use crate::chat::ChatLink;
use crate::home::HomeLink;

pub(super) fn spawn(
    link: &Arc<OnboardingLink>,
    runtime: &Handle,
    home: &Arc<HomeLink>,
    chat: &Arc<ChatLink>,
) {
    runtime.spawn(follow_home(Arc::downgrade(link), home.subscribe()));
    runtime.spawn(follow_chat(Arc::downgrade(link), chat.subscribe()));
    runtime.spawn(follow_sent(Arc::downgrade(link), chat.subscribe_sent()));
}

async fn follow_home(
    link: Weak<OnboardingLink>,
    mut home: Receiver<wonton_protocol::HomeViewModel>,
) {
    let mut server = None;
    let mut connected = false;
    loop {
        let view = home.borrow_and_update().clone();
        let next_server = view.server_id.clone();
        let Some(link) = link.upgrade() else { return };
        let server_changed = next_server != server;
        if server_changed {
            link.server_changed(next_server.clone());
            server = next_server;
        }
        let now_connected = view.status == HomeStatus::Connected;
        if now_connected && (!connected || server_changed) {
            link.refresh();
        }
        connected = now_connected;
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}

async fn follow_chat(
    link: Weak<OnboardingLink>,
    mut chat: Receiver<wonton_protocol::ChatViewModel>,
) {
    while chat.changed().await.is_ok() {
        let Some(link) = link.upgrade() else { return };
        let _ = chat.borrow_and_update();
        link.publish();
    }
}

async fn follow_sent(link: Weak<OnboardingLink>, mut sent: Receiver<Vec<String>>) {
    while sent.changed().await.is_ok() {
        let sent_rooms = sent.borrow_and_update().clone();
        let Some(link) = link.upgrade() else { return };
        // 房间在持锁时记下：记房间、写 `prompts_used`（publish 里）和渲染用同一把锁、同一份快照，
        // 这中间没有发布能看到“记下了房间、盘上还没有”的状态。
        let mut state = lock(&link.state);
        lock(&link.sent_rooms).extend(sent_rooms.iter().cloned());
        for room_id in &sent_rooms {
            if let Err(error) = link.storage.record_prompt_room(room_id) {
                state.problem = Some(storage_problem(error));
            }
        }
        drop(state);
        link.publish();
    }
}
