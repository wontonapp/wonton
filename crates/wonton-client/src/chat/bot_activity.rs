use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::{BotActivity, BotActivityFacts};

const MIN_HOLD: Duration = Duration::from_millis(1_500);
const IDLE_QUIET: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
pub(super) struct Activity {
    pub state: Option<BotActivity>,
    shown_since: Option<Instant>,
    quiet_since: Option<Instant>,
    pub next_change: Option<Instant>,
}

/// All priority and debounce decisions live here; callers supply the observation time.
pub(super) fn synthesize(
    previous: &Activity,
    facts: Option<&BotActivityFacts>,
    typing: bool,
    now: Instant,
) -> Activity {
    let Some(facts) = facts else {
        return Activity::default();
    };
    let target = if !facts.running {
        BotActivity::Offline
    } else if facts.waiting > 0 {
        BotActivity::Waiting
    } else if facts.model_requests > 0 || facts.tool_calls > 0 || typing {
        BotActivity::Working
    } else {
        BotActivity::Idle
    };
    let quiet_since = (target == BotActivity::Idle
        && matches!(
            previous.state,
            Some(BotActivity::Working | BotActivity::Waiting)
        ))
    .then(|| previous.quiet_since.unwrap_or(now));
    if previous.state == Some(target) {
        return Activity {
            quiet_since,
            next_change: None,
            ..previous.clone()
        };
    }
    if target != BotActivity::Offline && previous.state.is_some() {
        let hold_until = previous.shown_since.unwrap_or(now) + MIN_HOLD;
        let ready = quiet_since.map_or(hold_until, |since| (since + IDLE_QUIET).max(hold_until));
        if now < ready {
            return Activity {
                quiet_since,
                next_change: Some(ready),
                ..previous.clone()
            };
        }
    }
    Activity {
        state: Some(target),
        shown_since: Some(now),
        quiet_since: None,
        next_change: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::BotRef;

    fn facts() -> BotActivityFacts {
        BotActivityFacts {
            bot: BotRef {
                plugin_id: "bot".into(),
                bot: "default".into(),
            },
            user_id: "@bot:home".into(),
            running: true,
            model_requests: 0,
            tool_calls: 0,
            waiting: 0,
            profile_rev: 0,
        }
    }

    #[test]
    fn precedence_and_unknown_bypass_debounce() {
        let now = Instant::now();
        let mut facts = facts();
        facts.model_requests = 1;
        let working = synthesize(&Activity::default(), Some(&facts), true, now);
        assert_eq!(working.state, Some(BotActivity::Working));
        facts.waiting = 1;
        let held = synthesize(&working, Some(&facts), true, now + Duration::from_secs(1));
        assert_eq!(held.state, Some(BotActivity::Working));
        let waiting = synthesize(&held, Some(&facts), true, now + MIN_HOLD);
        assert_eq!(waiting.state, Some(BotActivity::Waiting));
        facts.running = false;
        let offline = synthesize(&waiting, Some(&facts), true, now + MIN_HOLD);
        assert_eq!(offline.state, Some(BotActivity::Offline));
        assert_eq!(synthesize(&offline, None, true, now).state, None);
    }

    #[test]
    fn idle_requires_continuous_quiet_and_busy_cancels_pending_idle() {
        let now = Instant::now();
        let mut facts = facts();
        facts.tool_calls = 1;
        let working = synthesize(&Activity::default(), Some(&facts), false, now);
        facts.tool_calls = 0;
        let quiet = synthesize(&working, Some(&facts), false, now + Duration::from_secs(3));
        assert_eq!(quiet.state, Some(BotActivity::Working));
        assert_eq!(quiet.next_change, Some(now + Duration::from_secs(5)));
        let still = synthesize(
            &quiet,
            Some(&facts),
            false,
            now + Duration::from_millis(4_999),
        );
        assert_eq!(still.state, Some(BotActivity::Working));
        let busy = synthesize(&still, Some(&facts), true, now + Duration::from_secs(5));
        assert_eq!(busy.state, Some(BotActivity::Working));
        assert_eq!(busy.next_change, None);
        let quiet = synthesize(&busy, Some(&facts), false, now + Duration::from_secs(6));
        assert_eq!(
            synthesize(&quiet, Some(&facts), false, now + Duration::from_secs(8)).state,
            Some(BotActivity::Idle)
        );
    }

    #[test]
    fn leaving_offline_holds_but_new_facts_after_unknown_are_immediate() {
        let now = Instant::now();
        let mut facts = facts();
        facts.running = false;
        let offline = synthesize(&Activity::default(), Some(&facts), false, now);
        facts.running = true;
        facts.waiting = 1;
        assert_eq!(
            synthesize(&offline, Some(&facts), false, now).state,
            Some(BotActivity::Offline)
        );
        assert_eq!(
            synthesize(&offline, Some(&facts), false, now + MIN_HOLD).state,
            Some(BotActivity::Waiting)
        );
        let unknown = synthesize(&offline, None, false, now);
        assert_eq!(
            synthesize(&unknown, Some(&facts), false, now).state,
            Some(BotActivity::Waiting)
        );
    }
}
