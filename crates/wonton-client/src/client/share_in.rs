//! 系统分享进云吞（ADR-0097）的入口。认领、发送、丢掉要知道成没成，是 `ClientCore` 的方法，不进 `Action`。

use std::sync::Arc;

use tokio::sync::{Mutex, watch};
use wonton_protocol::{
    ChatViewModel, CoreError, HomeStatus, HomeViewModel, OnboardingViewModel, ShareInImage,
    ShareInViewModel,
};

use super::ClientCore;
use crate::share_in::ShareInLink;

#[uniffi::export]
impl ClientCore {
    /// 认领 `<数据目录>/share-inbox/<令牌>` 里的一次分享，分享页随之出现在 [`subscribe_share`](Self::subscribe_share)
    /// 里；还没发的上一次被顶掉。Android 写好收件目录后直接调；iOS 由分享扩展写好后经 `wonton://share/<令牌>`
    /// （[`AppLink::Share`](wonton_protocol::AppLink::Share)）打开 App 再调。找不到、没写完、清单不合规则时返回
    /// `InvalidInput`（没写完、不合规则的目录随即删掉）；搬不动目录时返回 `Storage`。没配对也能认领。
    pub fn claim_share(&self, token: String) -> Result<(), CoreError> {
        self.share_in.claim(&token)
    }

    /// 把这次分享发到 `room_id`：字是主人在分享页输入框里的（可以为空），照片由壳从 [`ShareInItem::path`]
    /// （[`wonton_protocol::ShareInItem`]）读出来按从相册选的规则压缩后，按条目标识放进 `images`。和附件区一样变成这个房间
    /// 的一条待发消息（ADR-0094），不碰这个房间的附件区和草稿；成功后分享页收起，壳打开这个房间看进度。有照片没交回时
    /// 返回 `InvalidInput`；还没登录、房间不能发时同 `add_image_attachment`。
    pub fn send_share(
        &self,
        room_id: String,
        body: String,
        images: Vec<ShareInImage>,
    ) -> Result<(), CoreError> {
        let outbound = self.share_in.outbound(images)?;
        let token = outbound.token.clone();
        let has_files = outbound.has_files;
        self.scope()
            .composer
            .send_items(&room_id, &body, outbound.items)?;
        self.share_in.sent(&token, has_files, &room_id);
        Ok(())
    }

    /// 从分享页里删掉一项；`item_id` 为空时整个不发了。条目删光、也没有文字时同样收起分享页。
    pub fn discard_share(&self, item_id: Option<String>) -> Result<(), CoreError> {
        self.share_in.discard(item_id.as_deref())
    }

    /// 当前的分享页，用于首帧渲染。
    pub fn share_in(&self) -> ShareInViewModel {
        let views = self.views();
        let pending = self.share_in.subscribe();
        let pending = pending.borrow();
        self.share_in.view(
            pending.as_ref(),
            is_paired(&views.home.borrow()),
            &views.chat.borrow(),
            views.onboarding.borrow().primary_bot_user_id.as_deref(),
        )
    }

    /// 订阅分享页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。去向跟着连接、聊天和默认 Agent 变。
    pub fn subscribe_share(&self) -> Arc<ShareInSubscription> {
        let receivers = {
            let views = self.views();
            Receivers {
                pending: self.share_in.subscribe(),
                home: views.home.subscribe(),
                chat: views.chat.subscribe(),
                onboarding: views.onboarding.subscribe(),
            }
        };
        Arc::new(ShareInSubscription::new(self.share_in.clone(), receivers))
    }
}

fn is_paired(home: &HomeViewModel) -> bool {
    !matches!(home.status, HomeStatus::NotConfigured)
}

struct Receivers {
    pending: watch::Receiver<Option<crate::share_in::Claimed>>,
    home: watch::Receiver<HomeViewModel>,
    chat: watch::Receiver<ChatViewModel>,
    onboarding: watch::Receiver<OnboardingViewModel>,
}

impl Receivers {
    fn view(&mut self, link: &ShareInLink) -> ShareInViewModel {
        let pending = self.pending.borrow_and_update();
        let home = self.home.borrow_and_update();
        let chat = self.chat.borrow_and_update();
        let onboarding = self.onboarding.borrow_and_update();
        link.view(
            pending.as_ref(),
            is_paired(&home),
            &chat,
            onboarding.primary_bot_user_id.as_deref(),
        )
    }
}

/// 分享页的订阅，用法同 [`SessionListSubscription`](crate::SessionListSubscription)。
#[derive(uniffi::Object)]
pub struct ShareInSubscription {
    link: Arc<ShareInLink>,
    receivers: Mutex<Receivers>,
    last: Mutex<Option<ShareInViewModel>>,
    cancelled: watch::Sender<bool>,
}

impl ShareInSubscription {
    fn new(link: Arc<ShareInLink>, receivers: Receivers) -> Self {
        Self {
            link,
            receivers: Mutex::new(receivers),
            last: Mutex::new(None),
            cancelled: watch::Sender::new(false),
        }
    }
}

#[uniffi::export]
impl ShareInSubscription {
    /// 等待下一份 ViewModel：第一次立即返回当前值，之后在分享页变了时返回。订阅取消后返回 `None`。
    pub async fn next(&self) -> Option<ShareInViewModel> {
        let mut cancelled = self.cancelled.subscribe();
        let mut receivers = self.receivers.lock().await;
        let mut last = self.last.lock().await;
        loop {
            if *cancelled.borrow_and_update() {
                return None;
            }
            let view = receivers.view(&self.link);
            if last.as_ref() != Some(&view) {
                *last = Some(view.clone());
                return Some(view);
            }
            // 聊天的 ViewModel 变得很频繁（同步、未读数），分享页没变的那些不交给壳。
            let Receivers {
                pending,
                home,
                chat,
                onboarding,
            } = &mut *receivers;
            let changed = tokio::select! {
                biased;
                _ = cancelled.wait_for(|cancelled| *cancelled) => return None,
                changed = pending.changed() => changed,
                changed = home.changed() => changed,
                changed = chat.changed() => changed,
                changed = onboarding.changed() => changed,
            };
            changed.ok()?;
        }
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`](crate::SessionListSubscription::cancel)。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}
