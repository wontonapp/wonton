use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Wake};

use axum::http::Method;
use tokio::sync::oneshot;
use wonton_client::{
    Action, ChatAccount, ModelProviderFailure, OnboardingStep, OnboardingSubscription,
    OnboardingViewModel, SecretString,
};

use super::super::rig::{Rig, WAIT, core_error, running};
use super::{model_rig, onboarding, providers};

const ONBOARDING: &str = "/_wonton/api/onboarding";
const PROVIDERS: &str = "/_wonton/api/models/providers";
const KEY: &str = "/_wonton/api/models/providers/openai";

struct PausedWake {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Wake for PausedWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let entered = self.entered.lock().unwrap().take();
        if let Some(entered) = entered {
            entered.send(()).unwrap();
            // Hand the worker's queued network tasks to Tokio while the notification is paused.
            tokio::task::block_in_place(|| {
                let _ = self.release.lock().unwrap().recv_timeout(WAIT);
            });
        }
    }
}

type NextView = Pin<Box<dyn Future<Output = Option<OnboardingViewModel>> + Send>>;

struct RecordWake {
    subscription: Arc<OnboardingSubscription>,
    next: Mutex<Option<NextView>>,
    views: Mutex<Vec<OnboardingViewModel>>,
}

impl Wake for RecordWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let waker = self.clone().into();
        let mut context = Context::from_waker(&waker);
        let mut next = self.next.lock().unwrap();
        loop {
            let future = next.get_or_insert_with(|| {
                let subscription = self.subscription.clone();
                Box::pin(async move { subscription.next().await })
            });
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(Some(view)) => {
                    self.views.lock().unwrap().push(view);
                    *next = None;
                }
                _ => return,
            }
        }
    }
}

async fn wait_requests(rig: &Rig, method: Method, path: &str, count: usize) {
    tokio::time::timeout(WAIT, async {
        while rig.core.seen(&method, path).len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_delayed_chat_notification_cannot_restore_finished_loading() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("model"));
    rig.core.set_providers(providers(false));
    rig.core.hold(Method::GET, PROVIDERS);
    rig.connected().await;
    wait_requests(&rig, Method::GET, PROVIDERS, 1).await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    let chat = rig.app.subscribe_chat();
    tokio::time::timeout(WAIT, async {
        while !matches!(
            chat.next().await.unwrap().account,
            ChatAccount::Failed { .. }
        ) {}
    })
    .await
    .unwrap();

    let (entered, paused) = oneshot::channel();
    let (release, resume) = mpsc::channel();
    let waker = Arc::new(PausedWake {
        entered: Mutex::new(Some(entered)),
        release: Mutex::new(resume),
    })
    .into();
    let mut next_chat = pin!(chat.next());
    assert!(
        next_chat
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );

    let mut selected = onboarding("model");
    selected["primary_agent"] = serde_json::json!("agent.one");
    rig.core.set_onboarding(selected);
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.set_lists(vec![vec![running("agent.one")]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    tokio::time::timeout(WAIT, paused).await.unwrap().unwrap();

    rig.core.release(Method::GET, PROVIDERS);
    let ready = rig
        .wait_onboarding(|view| !view.loading && view.model.providers.is_some())
        .await;
    assert!(ready.nav.unwrap().can_skip);
    // Drain synchronously: watch may otherwise merge a stale frame with the next good frame.
    let recorded = Arc::new(RecordWake {
        subscription: rig.app.subscribe_onboarding(),
        next: Mutex::new(None),
        views: Mutex::new(Vec::new()),
    });
    recorded.wake_by_ref();
    release.send(()).unwrap();

    // follow_plugins handles changes serially: the next GET proves the paused publish returned.
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.set_lists(vec![vec![running("agent.two")]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 2).await;
    let finished = rig.app.onboarding();
    rig.core.release(Method::GET, ONBOARDING);
    drop(recorded.next.lock().unwrap().take());
    assert!(
        recorded
            .views
            .lock()
            .unwrap()
            .iter()
            .all(|view| !view.loading),
        "a completed load was published as loading again"
    );
    assert!(!finished.loading, "{finished:?}");
    assert!(finished.model.providers.is_some());
    assert!(finished.nav.unwrap().can_skip);
}

#[tokio::test]
async fn a_rejected_key_superseding_loading_reenables_navigation() {
    for (method, action) in [
        (Method::GET, Action::RefreshOnboarding),
        (Method::PUT, Action::SkipOnboardingStep),
    ] {
        let rig = model_rig().await;
        rig.core.hold(method.clone(), ONBOARDING);
        rig.app.dispatch(action).unwrap();
        assert!(rig.app.onboarding().loading);
        rig.core.answer(
            Method::PUT,
            KEY,
            core_error(422, "invalid_input", "bad key"),
        );
        rig.app
            .dispatch(Action::SaveModelProviderKey {
                id: "openai".to_owned(),
                key: SecretString::new("rejected"),
            })
            .unwrap();
        let failed = rig
            .wait_onboarding(|view| view.model.problem.is_some() && view.model.saving.is_none())
            .await;
        rig.core.release(method, ONBOARDING);
        assert_eq!(failed.step, Some(OnboardingStep::Model));
        assert_eq!(
            failed.model.problem.unwrap().reason,
            ModelProviderFailure::KeyRejected
        );
        assert!(!failed.loading);
        assert!(failed.nav.unwrap().can_skip);
    }
}

#[tokio::test]
async fn a_failed_step_superseding_a_key_save_reenables_navigation() {
    let rig = model_rig().await;
    rig.core.hold(Method::PUT, KEY);
    rig.app
        .dispatch(Action::SaveModelProviderKey {
            id: "openai".to_owned(),
            key: SecretString::new("secret"),
        })
        .unwrap();
    wait_requests(&rig, Method::PUT, KEY, 1).await;
    rig.core.answer(
        Method::PUT,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "step unavailable"),
    );
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    let failed = rig
        .wait_onboarding(|view| view.problem.is_some() && !view.loading)
        .await;
    rig.core.release(Method::PUT, KEY);
    assert_eq!(failed.step, Some(OnboardingStep::Model));
    assert!(failed.model.saving.is_none());
    assert!(failed.nav.unwrap().can_skip);
}
