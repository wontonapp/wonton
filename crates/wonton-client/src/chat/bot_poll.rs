use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;
use wonton_protocol::{BOT_ACTIVITY_WAIT_SECS, BotActivityReport, HomeViewModel, PluginProblem};

use super::bots::Bots;
use super::lock;

const TIMEOUT: Duration = Duration::from_secs(BOT_ACTIVITY_WAIT_SECS + 10);
const RETRY: Duration = Duration::from_secs(3);

pub(super) async fn follow(
    bots: Weak<Bots>,
    mut home: watch::Receiver<HomeViewModel>,
    mut foreground: watch::Receiver<bool>,
) {
    let mut after = None;
    let Some(link) = bots.upgrade() else { return };
    let mut session = link.session.subscribe();
    drop(link);
    loop {
        let Some(link) = bots.upgrade() else { return };
        if !link.can_poll() {
            link.clear_activity();
            after = None;
            drop(link);
            tokio::select! {
                result = home.changed() => if result.is_err() { return },
                result = foreground.changed() => if result.is_err() { return },
                result = session.changed() => if result.is_err() { return },
            }
            continue;
        }
        let target = link.home.view();
        let epoch = link.epoch.load(std::sync::atomic::Ordering::SeqCst);
        let generation = lock(&link.local).web_generation;
        let request = fetch(link.clone(), after);
        tokio::pin!(request);
        let result = loop {
            let deadline = link.update_activity();
            tokio::select! {
                result = &mut request => break Some(result),
                () = due(deadline) => {},
                () = link.changed.notified() => {},
                result = home.changed() => {
                    if result.is_err() { return; }
                    let next = home.borrow_and_update().clone();
                    if !link.can_poll() || next.server_id != target.server_id || next.proxy != target.proxy {
                        break None;
                    }
                },
                result = foreground.changed() => {
                    if result.is_err() { return; }
                    break None;
                },
                result = session.changed() => {
                    if result.is_err() { return; }
                    break None;
                },
            }
        };
        match result {
            Some(Ok(report)) => {
                let current = link.home.view();
                if !link.can_poll()
                    || current.server_id != target.server_id
                    || current.proxy != target.proxy
                    || link.epoch.load(std::sync::atomic::Ordering::SeqCst) != epoch
                {
                    link.clear_activity();
                    after = None;
                    continue;
                }
                after = Some(report.rev);
                apply(&link, report, epoch, generation);
            }
            Some(Err(error)) => {
                tracing::info!(?error, "Cannot poll Bot activity");
                link.clear_activity();
                after = None;
                drop(link);
                let retry_at = Instant::now() + RETRY;
                loop {
                    tokio::select! {
                        () = tokio::time::sleep_until(retry_at) => break,
                        result = home.changed() => if result.is_err() { return },
                        result = foreground.changed() => if result.is_err() { return },
                        result = session.changed() => if result.is_err() { return },
                    }
                    let Some(link) = bots.upgrade() else { return };
                    if !link.can_poll() {
                        break;
                    }
                }
            }
            None => {
                link.clear_activity();
                after = None;
            }
        }
    }
}

async fn fetch(bots: Arc<Bots>, after: Option<u64>) -> Result<BotActivityReport, PluginProblem> {
    let after = after.map(|rev| rev.to_string());
    let query = after.as_deref().map(|rev| [("after", rev)]);
    bots.plugins
        .api()?
        .get(
            &["bots", "activity"],
            query.as_ref().map_or(&[], |query| query.as_slice()),
            TIMEOUT,
        )
        .await
}

async fn due(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn apply(bots: &Arc<Bots>, report: BotActivityReport, epoch: u64, generation: u64) {
    let mut local = lock(&bots.local);
    if bots.epoch.load(std::sync::atomic::Ordering::SeqCst) != epoch
        || local.web_generation != generation
        || !bots.can_poll()
    {
        return;
    }
    local.report = Some(report);
    drop(local);
    bots.reconcile();
}
