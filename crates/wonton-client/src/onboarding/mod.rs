//! 新用户引导：每一步记下的选择、最后一页的提交和设备上的一次性提示（ADR-0034、ADR-0038）。

mod api;
mod draft;
mod follow;
mod model;
mod refresh;
mod review;
mod steps;
mod storage;
mod view;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;
use tokio::sync::{watch, watch::Receiver};
use wonton_protocol::{
    Action, CoreError, HomeStatus, ModelProviderFailure, ModelProviderProblem, ModelProviders,
    Onboarding, OnboardingCommit, OnboardingStep, OnboardingTip, OnboardingUpdate,
    OnboardingViewModel, OwnerProfile, PluginFailure, PluginProblem,
};

use crate::chat::ChatLink;
use crate::home::HomeLink;
use crate::plugins::PluginsLink;
use draft::Draft;
use review::ReviewEntry;
use storage::OnboardingStorage;
use view::render;

#[derive(Default)]
struct LocalState {
    /// 家里服务器上的引导状态：只记走没走完和默认 Agent。
    onboarding: Option<Onboarding>,
    revision: u64,
    /// 正在显示的一步。中间的步骤只在这台设备上（ADR-0038）；还没取到时为空。
    step: Option<OnboardingStep>,
    owner: Option<OwnerProfile>,
    providers: Option<ModelProviders>,
    draft: Draft,
    testing: bool,
    review: Vec<ReviewEntry>,
    /// 每次进出最后一页加一，旧的预览结果不落到新列出的项上。
    review_generation: u64,
    commit: OnboardingCommit,
    problem: Option<PluginProblem>,
    model_problem: Option<ModelProviderProblem>,
    loading: bool,
}

pub(crate) struct OnboardingLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    chat: Arc<ChatLink>,
    plugins: Arc<PluginsLink>,
    view: watch::Sender<OnboardingViewModel>,
    state: Mutex<LocalState>,
    storage: OnboardingStorage,
    /// 上次告诉核心的语言，变了才重新取：家里的服务器按 `Accept-Language` 挑引导里的名字、问题和服务商。
    languages: Mutex<Vec<String>>,
    /// 换服务器时加一，之前发起的请求的结果都作废。
    generation: AtomicU64,
    /// 每次取状态或者在设置里改引导状态时加一，只认最后一次的结果。
    operation: AtomicU64,
    sent_rooms: Mutex<HashSet<String>>,
}

impl OnboardingLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        chat: Arc<ChatLink>,
        plugins: Arc<PluginsLink>,
        data_dir: &Path,
    ) -> Arc<Self> {
        let (storage, problem) = OnboardingStorage::open(data_dir);
        let sent_rooms = storage.snapshot().prompt_rooms.into_iter().collect();
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.clone(),
            chat: chat.clone(),
            plugins,
            view: watch::Sender::new(OnboardingViewModel::default()),
            state: Mutex::new(LocalState {
                problem: problem.map(storage_problem),
                ..LocalState::default()
            }),
            storage,
            languages: Mutex::new(Vec::new()),
            generation: AtomicU64::new(0),
            operation: AtomicU64::new(0),
            sent_rooms: Mutex::new(sent_rooms),
        });
        link.publish();
        follow::spawn(&link, &runtime, &home, &chat);
        runtime.spawn(refresh::follow_plugins(
            Arc::downgrade(&link),
            link.plugins.subscribe(),
        ));
        link
    }

    pub(crate) fn view(&self) -> OnboardingViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> Receiver<OnboardingViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        let changed = {
            let mut current = lock(&self.languages);
            if *current == languages {
                false
            } else {
                *current = languages;
                true
            }
        };
        if changed && self.home.view().status == HomeStatus::Connected {
            self.refresh();
        } else if changed {
            self.publish();
        }
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::RefreshOnboarding => {
                self.refresh();
                Ok(())
            }
            Action::RestartOnboarding => self.restart(),
            Action::ChooseOnboardingAgent { id } => self.choose_agent(&id),
            Action::ContinueOnboarding => self.continue_step(),
            Action::SkipOnboardingStep => self.skip_step(),
            Action::BackOnboardingStep => self.back_step(),
            Action::ToggleOnboardingNeed { id } => self.toggle_need(&id),
            Action::EditOnboardingProfile {
                display_name,
                email,
            } => self.edit_profile(display_name, email),
            Action::TestModelProvider {
                input,
                then_continue,
            } => self.test_provider(input, then_continue),
            Action::UseCurrentModel => self.use_current_model(),
            Action::ChooseModelLogin { id } => self.choose_login(&id),
            Action::RetryOnboardingItem { id } => self.retry_item(&id),
            Action::DismissModelProviderProblem => {
                lock(&self.state).model_problem = None;
                self.publish();
                Ok(())
            }
            Action::SetPrimaryAgent { id } => self.set_primary(&id),
            Action::DismissReplacedAgentOffer => self.dismiss_replaced(),
            Action::DismissOnboardingTip { tip } => self.dismiss_tip(tip),
            _ => Err(invalid("不是新用户引导操作")),
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn begin_operation(&self) -> (u64, u64) {
        (
            self.generation(),
            self.operation.fetch_add(1, Ordering::SeqCst) + 1,
        )
    }

    fn operation_is_current(&self, generation: u64, operation: u64) -> bool {
        self.generation() == generation && self.operation.load(Ordering::SeqCst) == operation
    }

    /// 重新取家里服务器上的状态。记下的选择和正在显示的一步不变：从网关网页回来、出错后重试都走这里。
    fn refresh(self: &Arc<Self>) {
        let (generation, operation) = {
            let mut state = lock(&self.state);
            state.loading = true;
            state.problem = None;
            state.model_problem = None;
            self.begin_operation()
        };
        self.publish();
        let link = self.clone();
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            let result = match link.plugins.api() {
                Ok(api) => api::get(&api).await.map(|onboarding| (api, onboarding)),
                Err(problem) => Err(problem),
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            let (api, onboarding) = match result {
                Ok(fetched) => fetched,
                Err(problem) => return link.finish_error(generation, operation, problem),
            };
            let in_flow = {
                let state = lock(&link.state);
                let step = state.step.unwrap_or(onboarding.step);
                !matches!(step, OnboardingStep::Finished | OnboardingStep::Other)
            };
            // 走完了的不取资料和服务商：没有地方显示它们，重看引导时再取。
            let details = if in_flow {
                Some(tokio::join!(api::owner_profile(&api), api::providers(&api)))
            } else {
                None
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            link.received(onboarding, details);
        });
    }

    fn received(
        &self,
        onboarding: Onboarding,
        details: Option<(
            Result<OwnerProfile, PluginProblem>,
            Result<ModelProviders, ModelProviderProblem>,
        )>,
    ) {
        {
            let mut state = lock(&self.state);
            if state.step.is_none() {
                // 中间的步骤不存在家里服务器上：没走完的从第一步开始，每一步显示现在的设置。
                state.step = Some(match onboarding.step {
                    OnboardingStep::Finished => OnboardingStep::Finished,
                    OnboardingStep::Other => OnboardingStep::Other,
                    _ => OnboardingStep::Profile,
                });
            }
            state.onboarding = Some(onboarding);
            state.revision += 1;
            if let Some((owner, providers)) = details {
                match owner {
                    Ok(owner) => state.owner = Some(owner),
                    Err(problem) => state.problem = Some(problem),
                }
                match providers {
                    Ok(providers) => state.providers = Some(providers),
                    Err(problem) => state.model_problem = Some(problem),
                }
            }
            state.loading = false;
        }
        self.publish();
        self.maybe_mark_prompt_used();
    }

    fn finish_error(&self, generation: u64, operation: u64, problem: PluginProblem) {
        if !self.operation_is_current(generation, operation) {
            return;
        }
        let mut state = lock(&self.state);
        state.loading = false;
        state.problem = Some(problem);
        drop(state);
        self.publish();
    }

    fn set_primary(self: &Arc<Self>, id: &str) -> Result<(), CoreError> {
        let state = lock(&self.state);
        let onboarding = state
            .onboarding
            .as_ref()
            .ok_or_else(|| invalid("还没有取到引导状态"))?;
        let agent = onboarding
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .ok_or_else(|| invalid("这个 Agent 不在列表里"))?;
        if !view::installable(agent) {
            return Err(invalid("这个 Agent 不能安装"));
        }
        drop(state);
        self.spawn_update(OnboardingUpdate {
            primary_agent: Some(id.to_owned()),
            ..OnboardingUpdate::default()
        });
        Ok(())
    }

    fn dismiss_replaced(self: &Arc<Self>) -> Result<(), CoreError> {
        if lock(&self.state).onboarding.is_none() {
            return Err(invalid("还没有取到引导状态"));
        }
        self.spawn_update(OnboardingUpdate {
            replaced_agent_asked: Some(true),
            ..OnboardingUpdate::default()
        });
        Ok(())
    }

    fn dismiss_tip(&self, tip: OnboardingTip) -> Result<(), CoreError> {
        self.storage.dismiss_tip(tip)?;
        // 写成功说明本机记录已经完整，启动时“读不了记录”的问题不再成立。
        lock(&self.state).problem = None;
        self.publish();
        Ok(())
    }

    /// 设置里的改动（默认 Agent、要不要卸掉换下来的）直接保存，不走引导最后一起提交。
    fn spawn_update(self: &Arc<Self>, update: OnboardingUpdate) {
        let (generation, operation) = {
            let mut state = lock(&self.state);
            state.loading = true;
            state.problem = None;
            self.begin_operation()
        };
        self.publish();
        let link = self.clone();
        self.runtime.spawn(async move {
            let result = match link.plugins.api() {
                Ok(api) => api::update(&api, &update).await,
                Err(problem) => Err(problem),
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            let mut state = lock(&link.state);
            state.loading = false;
            match result {
                Ok(onboarding) => {
                    state.onboarding = Some(onboarding);
                    state.revision += 1;
                }
                Err(problem) => state.problem = Some(problem),
            }
            drop(state);
            link.publish();
        });
    }

    fn server_changed(&self, server_id: Option<String>) {
        {
            let mut state = lock(&self.state);
            self.generation.fetch_add(1, Ordering::SeqCst);
            *state = LocalState::default();
        }
        lock(&self.sent_rooms).clear();
        if let Some(server_id) = server_id
            && let Err(error) = self.storage.sync_server(&server_id)
        {
            lock(&self.state).problem = Some(storage_problem(error));
        }
        self.publish();
    }

    fn publish(&self) {
        let state = lock(&self.state);
        let records = self.storage.snapshot();
        let chat = self.chat.view();
        let sent_rooms = lock(&self.sent_rooms).clone();
        let (view, primary_bot) = render(&state, &records, &chat, &sent_rooms);
        // 快照必须在持锁时发布，不能让较早渲染的 loading 覆盖完成状态。
        self.view.send_replace(view);
        drop(state);
        self.chat.set_primary_bot(primary_bot);
    }
}

fn provider_problem(problem: PluginProblem) -> ModelProviderProblem {
    ModelProviderProblem {
        reason: match problem.reason {
            PluginFailure::TransportUnavailable => ModelProviderFailure::TransportUnavailable,
            PluginFailure::Unauthorized => ModelProviderFailure::Unauthorized,
            PluginFailure::Protocol => ModelProviderFailure::Protocol,
            _ => ModelProviderFailure::Upstream,
        },
        message: problem.message,
    }
}

fn storage_problem(error: CoreError) -> PluginProblem {
    PluginProblem {
        reason: PluginFailure::Upstream,
        message: error.to_string(),
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
