use std::collections::HashSet;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, ThreadId};

use axum::http::Method;
use tokio::sync::oneshot;
use wonton_client::{
    Action, ChatAccount, ChatSubscription, ChatViewModel, OnboardingSubscription,
    OnboardingViewModel,
};

use super::super::rig::{Rig, WAIT, running};
use super::{MODEL_SETTINGS, ONBOARDING, model_settings, onboarding, wait_requests};

/// `onboarding()` 里 agent.one 的 Bot：选它当默认 Agent 后，引导发布把它交给聊天，聊天视图随之通知一次。
const BOT: &str = "@agent:home";

type NextChat = Pin<Box<dyn Future<Output = Option<ChatViewModel>> + Send>>;

/// 只暂停引导发布交出默认 Bot 的那次聊天通知。聊天视图还有别的发送方：插件列表每变一次，安装占位行和
/// Bot 列表都会无条件推一次，安装占位行推的时候还持着自己的锁。暂停到它们身上时，等这把锁的引导发布会占住
/// 客户端运行时仅有的两个工作线程，刷新引导的 GET 被拖到测试再次扣住之后，测试就等不到下一个 GET。
/// 所以别的线程叫醒时只重新登记，只有引导发布所在的线程才暂停。
struct PausedWake {
    chat: Arc<ChatSubscription>,
    next: Mutex<Option<NextChat>>,
    publisher: Arc<Mutex<Option<ThreadId>>>,
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Wake for PausedWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if *self.publisher.lock().unwrap() == Some(thread::current().id()) {
            let entered = self.entered.lock().unwrap().take();
            if let Some(entered) = entered {
                entered.send(()).unwrap();
                // Hand the worker's queued network tasks to Tokio while the notification is paused.
                tokio::task::block_in_place(|| {
                    let _ = self.release.lock().unwrap().recv_timeout(WAIT);
                });
                return;
            }
        }
        self.listen();
    }
}

impl PausedWake {
    /// 取走已经到了的聊天视图，直到再次挂起：waker 一直登记在聊天视图上。
    fn listen(self: &Arc<Self>) {
        let waker = Waker::from(self.clone());
        let mut context = Context::from_waker(&waker);
        let mut next = self.next.lock().unwrap();
        loop {
            let future = next.get_or_insert_with(|| {
                let chat = self.chat.clone();
                Box::pin(async move { chat.next().await })
            });
            match future.as_mut().poll(&mut context) {
                Poll::Ready(Some(_)) => *next = None,
                _ => return,
            }
        }
    }
}

/// 刷新插件列表，引导跟着取到选了 agent.one 的状态，在它的发布交出默认 Bot 时暂停。取状态先扣住，
/// 插件列表这一轮引起的聊天通知都在引导发布之前发完。返回放行用的发送端和这之前取过几次状态。
async fn pause_selecting_publish(
    rig: &Rig,
    chat: Arc<ChatSubscription>,
) -> (mpsc::Sender<()>, usize) {
    let publisher = Arc::new(Mutex::new(None));
    let marker = publisher.clone();
    rig.app.test_set_onboarding_publish_hook(Some(Arc::new(
        move |view: &OnboardingViewModel, _: &HashSet<String>| {
            if view.primary_bot_user_id.as_deref() == Some(BOT) {
                marker
                    .lock()
                    .unwrap()
                    .get_or_insert_with(|| thread::current().id());
            }
        },
    )));
    let (entered, paused) = oneshot::channel();
    let (release, resume) = mpsc::channel();
    let pause = Arc::new(PausedWake {
        chat,
        next: Mutex::new(None),
        publisher,
        entered: Mutex::new(Some(entered)),
        release: Mutex::new(resume),
    });

    let mut selected = onboarding("model");
    selected["primary_agent"] = serde_json::json!("agent.one");
    rig.core.set_onboarding(selected);
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.set_lists(vec![vec![running("agent.one")]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait_requests(rig, Method::GET, ONBOARDING, gets + 1).await;
    pause.listen();
    rig.core.release(Method::GET, ONBOARDING);
    tokio::time::timeout(WAIT, paused).await.unwrap().unwrap();
    (release, gets)
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
                Poll::Ready(Some(view)) => {
                    self.views.lock().unwrap().push(view);
                    *next = None;
                }
                _ => return,
            }
        }
    }
}

#[tokio::test]
async fn a_delayed_chat_notification_cannot_restore_finished_loading() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("model"));
    rig.core.set_model_settings(model_settings(false));
    rig.connected().await;
    rig.wait_onboarding(|view| view.step.is_some() && !view.loading)
        .await;
    // 第二次取状态卡在模型设置上：onboarding 已经有了，loading 还没结束。
    rig.core.hold(Method::GET, MODEL_SETTINGS);
    rig.app
        .dispatch(Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        })
        .unwrap();
    wait_requests(&rig, Method::GET, MODEL_SETTINGS, 2).await;
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
    let (release, gets) = pause_selecting_publish(&rig, chat).await;

    rig.core.release(Method::GET, MODEL_SETTINGS);
    let ready = rig
        .wait_onboarding(|view| !view.loading && view.model.settings.is_some())
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
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 2).await;
    let finished = rig.app.onboarding();
    rig.core.release(Method::GET, ONBOARDING);
    rig.app.test_set_onboarding_publish_hook(None);
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
    assert!(finished.model.settings.is_some());
    assert!(finished.nav.unwrap().can_skip);
}
