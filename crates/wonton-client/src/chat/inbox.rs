//! 收件箱（ADR-0119）：长轮询核心的收件箱接口，和确认卡片列表（[`super::pending`]）合成壳要画的
//! [`InboxViewModel`]。确认卡片在核心的收件箱里是 `confirm` 条目（`c-<卡片 id>`），只带摘要；能回答的卡片本身
//! 来自确认卡片列表，这里按卡片 ID 合进条目。
//!
//! 没有要回答的东西时 `counts.needing` 是 0，铃铛不画数字；卡片答完、过期、被撤回，核心的列表跟着变，计数
//! 跟着减。取不到时留上次的列表、如实带着问题，退避重试；[`ChatAction::RetryInbox`](wonton_protocol::ChatAction::RetryInbox)
//! 立即重试。

use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::watch;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    ChatFailure, ChatProblem, ConfirmationView, ConfirmationsViewModel, HomeViewModel, INBOX_PATH,
    INBOX_WAIT_SECS, InboxEntry, InboxList, InboxType, InboxViewModel, ProxySettings,
};

use super::ChatLink;
use super::confirmation::{error_message, failure_reason, tunnel_client};
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};

/// 长轮询最多挂 [`INBOX_WAIT_SECS`]，客户端的超时要比它长。
const POLL_TIMEOUT: Duration = Duration::from_secs(INBOX_WAIT_SECS + 10);
const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// 收件箱条目 ID 的确认卡片前缀（核心的投影：`c-<卡片 id>`）。
const CONFIRM_PREFIX: &str = "c-";

pub(super) async fn follow_inbox(chat: Weak<ChatLink>, home: watch::Receiver<HomeViewModel>) {
    let Some(link) = chat.upgrade() else { return };
    let mut follower = Follower {
        out: link.inbox.clone(),
        cards: link.confirmations.subscribe(),
        retry: link.retry_inbox.clone(),
        home_link: link.home.clone(),
        home,
        latest: None,
        after: None,
        problem: None,
        backoff: Duration::ZERO,
    };
    drop(link);
    follower.publish();
    follower.run().await;
}

struct Follower {
    out: watch::Sender<InboxViewModel>,
    cards: watch::Receiver<ConfirmationsViewModel>,
    retry: Arc<tokio::sync::Notify>,
    home_link: Arc<HomeLink>,
    home: watch::Receiver<HomeViewModel>,
    latest: Option<InboxList>,
    after: Option<u64>,
    problem: Option<ChatProblem>,
    backoff: Duration,
}

impl Follower {
    async fn run(mut self) {
        loop {
            let Some((core_url, proxy)) = self.wait_ready().await else {
                return;
            };
            let mut task = tokio::spawn(fetch(core_url, proxy, self.after));
            let fetched = loop {
                tokio::select! {
                    fetched = &mut task => break fetched.unwrap_or_else(|error| {
                        Err(transport(format!("the inbox request failed: {error}")))
                    }),
                    changed = self.cards.changed() => {
                        if changed.is_err() { return; }
                        self.publish();
                    }
                    _ = self.retry.notified() => self.backoff = Duration::ZERO,
                    changed = self.home.changed() => {
                        if changed.is_err() { return; }
                    }
                }
            };
            match fetched {
                Ok(list) => {
                    self.after = Some(list.revision);
                    self.latest = Some(list);
                    self.problem = None;
                    self.backoff = Duration::ZERO;
                }
                Err(problem) => {
                    self.problem = Some(problem);
                    self.backoff = if self.backoff.is_zero() {
                        BACKOFF_START
                    } else {
                        (self.backoff * 2).min(BACKOFF_MAX)
                    };
                }
            }
            self.publish();
        }
    }

    /// 连接没就绪就不打核心；失败过的先等退避，`RetryInbox` 立即放行。等的时候卡片列表变了也就地重算。
    async fn wait_ready(&mut self) -> Option<(Url, ProxySettings)> {
        if !self.backoff.is_zero() {
            let sleep = tokio::time::sleep(self.backoff);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    () = &mut sleep => break,
                    _ = self.retry.notified() => break,
                    changed = self.home.changed() => { changed.ok()?; break; }
                    changed = self.cards.changed() => { changed.ok()?; self.publish(); }
                }
            }
        }
        loop {
            let proxy = self.home.borrow_and_update().proxy.clone();
            if let (Some(proxy), Some(core_url)) = (proxy, self.home_link.core_url()) {
                return Some((core_url, proxy));
            }
            tokio::select! {
                changed = self.home.changed() => { changed.ok()?; }
                _ = self.retry.notified() => {}
                changed = self.cards.changed() => { changed.ok()?; self.publish(); }
            }
        }
    }

    fn publish(&mut self) {
        let cards = self.cards.borrow_and_update().clone();
        let view = compose(self.latest.as_ref(), &cards, self.problem.clone());
        self.out.send_if_modified(|current| {
            if *current == view {
                return false;
            }
            *current = view;
            true
        });
    }
}

/// 核心的收件箱列表加确认卡片列表，合成壳画的那一份。
pub(crate) fn compose(
    list: Option<&InboxList>,
    cards: &ConfirmationsViewModel,
    problem: Option<ChatProblem>,
) -> InboxViewModel {
    let card_of = |item_id: &str| -> Option<ConfirmationView> {
        let card_id = item_id.strip_prefix(CONFIRM_PREFIX)?;
        cards
            .pending
            .iter()
            .chain(&cards.finished)
            .find(|view| view.card.id == card_id)
            .cloned()
    };
    let loading = list.is_none() || cards.loading;
    let mut list = list.cloned().unwrap_or_default();
    let entries = list
        .items
        .iter()
        .map(|item| InboxEntry {
            card: (item.kind == InboxType::Confirm)
                .then(|| card_of(&item.id))
                .flatten(),
            item: item.clone(),
        })
        .collect();
    let contact_requests = cards.contact_requests.clone();
    list.counts.needing = list
        .counts
        .needing
        .saturating_add(u32::try_from(contact_requests.len()).unwrap_or(u32::MAX));
    InboxViewModel {
        entries,
        contact_requests,
        loading,
        problem: problem.or_else(|| cards.problem.clone()),
        list,
    }
}

fn transport(message: String) -> ChatProblem {
    failed(ChatFailure::TransportUnavailable, message)
}

fn failed(reason: ChatFailure, message: String) -> ChatProblem {
    ChatProblem {
        reason,
        message: l10n!(
            CATALOG,
            language(),
            "chat-inbox-list-failed",
            message = message
        ),
    }
}

/// `GET <核心地址>/_wonton/api/inbox?after=<revision>`：核心没有更新的就挂住，所以超时比等待长。
async fn fetch(
    core_url: Url,
    proxy: ProxySettings,
    after: Option<u64>,
) -> Result<InboxList, ChatProblem> {
    let mut url = core_url
        .join(INBOX_PATH)
        .map_err(|error| failed(ChatFailure::InvalidInput, error.to_string()))?;
    if let Some(after) = after {
        url.query_pairs_mut()
            .append_pair("after", &after.to_string());
    }
    let client = tunnel_client(&proxy, POLL_TIMEOUT)
        .map_err(|error| failed(ChatFailure::Upstream, error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| transport(error.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .map_err(|error| transport(error.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(failed(failure_reason(status), error_message(status, &body)));
    }
    serde_json::from_slice(&body).map_err(|error| {
        failed(
            ChatFailure::Upstream,
            l10n!(
                CATALOG,
                language(),
                "chat-confirmation-response-unreadable",
                error = error.to_string()
            ),
        )
    })
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod inbox_tests;
