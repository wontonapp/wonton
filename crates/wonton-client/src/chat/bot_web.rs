use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{BotRef, BotWebFailure, BotWebView, HomeViewModel};

use super::bots::Bots;
use super::lock;
use crate::plugins::api::{CoreApiError, READ_TIMEOUT};

struct Request {
    bot: BotRef,
    user: String,
    identity: u64,
    generation: u64,
    epoch: u64,
    home: HomeViewModel,
}

impl Bots {
    pub(super) fn refresh_web(self: &Arc<Self>) {
        if !self.can_poll() || self.session.borrow().is_none() {
            return;
        }
        let Ok(api) = self.plugins.api() else {
            self.clear_activity();
            return;
        };
        let mut local = lock(&self.local);
        local.web_generation = local.web_generation.wrapping_add(1);
        let generation = local.web_generation;
        let epoch = self.epoch.load(Ordering::SeqCst);
        let home = self.home.view();
        for (bot, record) in &local.records {
            let request = Request {
                bot: bot.clone(),
                user: record.user_id.clone(),
                identity: record.identity,
                generation,
                epoch,
                home: home.clone(),
            };
            let bots = self.clone();
            let api = api.clone();
            self.runtime.spawn(async move {
                let result = api
                    .get_detailed::<BotWebView>(
                        &["bots", &request.bot.plugin_id, &request.bot.bot, "webui"],
                        &[],
                        READ_TIMEOUT,
                    )
                    .await;
                let result = match result {
                    Ok(view) => Some(view),
                    Err(error) => failure(error),
                };
                bots.apply_web(&request, result);
            });
        }
    }

    fn apply_web(&self, request: &Request, result: Option<BotWebView>) {
        let local = lock(&self.local);
        let home = self.home.view();
        if !requester_current(
            request,
            &home,
            self.epoch.load(Ordering::SeqCst),
            *self.foreground.borrow(),
            self.session.borrow().is_some(),
        ) || !current(&local, request)
        {
            return;
        }
        self.view
            .send_if_modified(|view| apply(view, &local, request, result));
    }
}

fn requester_current(
    request: &Request,
    home: &HomeViewModel,
    epoch: u64,
    foreground: bool,
    signed_in: bool,
) -> bool {
    foreground
        && signed_in
        && home.status == wonton_protocol::HomeStatus::Connected
        && epoch == request.epoch
        && home.server_id == request.home.server_id
        && home.proxy == request.home.proxy
}

fn current(local: &super::bots::Local, request: &Request) -> bool {
    local.web_generation == request.generation
        && local.records.get(&request.bot).is_some_and(|record| {
            record.identity == request.identity
                && record.user_id == request.user
                && record.facts.is_some()
        })
}

fn apply(
    view: &mut wonton_protocol::ChatViewModel,
    local: &super::bots::Local,
    request: &Request,
    result: Option<BotWebView>,
) -> bool {
    if !current(local, request) {
        return false;
    }
    let result = exact(&request.bot, result);
    let Some(bot) = view
        .bots
        .iter_mut()
        .find(|bot| bot.bot == request.bot && bot.user_id == request.user)
    else {
        return false;
    };
    if bot.web == result {
        return false;
    }
    bot.web = result;
    true
}

fn exact(bot: &BotRef, result: Option<BotWebView>) -> Option<BotWebView> {
    match result {
        Some(BotWebView::Ready { page }) if page.bot == *bot => Some(BotWebView::Ready { page }),
        Some(BotWebView::Unavailable { reason }) => Some(BotWebView::Unavailable { reason }),
        _ => None,
    }
}

fn failure(error: CoreApiError) -> Option<BotWebView> {
    let CoreApiError::Http {
        kind: Some(kind), ..
    } = error
    else {
        return None;
    };
    let reason = match kind.as_str() {
        "unauthenticated" => BotWebFailure::Unauthenticated,
        "not_shared" => BotWebFailure::NotShared,
        "permission_denied" => BotWebFailure::PermissionDenied,
        "not_found" => BotWebFailure::NotFound,
        "no_webui" => BotWebFailure::NoWebui,
        "not_ready" => BotWebFailure::NotReady,
        _ => return None,
    };
    Some(BotWebView::Unavailable { reason })
}

#[cfg(test)]
#[path = "bot_web_tests.rs"]
mod tests;
