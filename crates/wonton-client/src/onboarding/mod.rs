//! 新用户引导：步骤状态、模型服务商和设备上的一次性提示（ADR-0034）。

mod api;
mod refresh;
mod storage;
mod text;
mod view;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use tokio::runtime::Handle;
use tokio::sync::{watch, watch::Receiver};
use wonton_protocol::{
    Action, CoreError, HomeStatus, ModelProviderFailure, ModelProviderKey, ModelProviderProblem,
    ModelProviders, Onboarding, OnboardingStep, OnboardingTip, OnboardingUpdate,
    OnboardingViewModel, PluginFailure, PluginProblem,
};

use crate::chat::ChatLink;
use crate::home::HomeLink;
use crate::plugins::PluginsLink;
use storage::OnboardingStorage;
use view::{next_step, render, selected_agent};

#[derive(Clone, Copy)]
enum Direction {
    Forward,
    Backward,
}

#[derive(Default)]
struct LocalState {
    onboarding: Option<Onboarding>,
    revision: u64,
    providers: Option<ModelProviders>,
    selected_agent: Option<String>,
    selected_needs: HashSet<String>,
    problem: Option<PluginProblem>,
    model_problem: Option<ModelProviderProblem>,
    saving: Option<String>,
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
    languages: Mutex<Vec<String>>,
    generation: AtomicU64,
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
        runtime.spawn(follow_home(Arc::downgrade(&link), home.subscribe()));
        runtime.spawn(follow_chat(Arc::downgrade(&link), chat.subscribe()));
        runtime.spawn(follow_sent(Arc::downgrade(&link), chat.subscribe_sent()));
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
            Action::ContinueOnboarding => self.continue_step(false),
            Action::SkipOnboardingStep => self.continue_step(true),
            Action::BackOnboardingStep => self.back_step(),
            Action::ToggleOnboardingNeed { id } => self.toggle_need(&id),
            Action::SaveModelProviderKey { id, key } => self.save_provider(&id, key),
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
    fn begin_operation(&self) -> (u64, u64) {
        let _state = lock(&self.state);
        (
            self.generation.load(Ordering::SeqCst),
            self.operation.fetch_add(1, Ordering::SeqCst) + 1,
        )
    }

    fn operation_is_current(&self, generation: u64, operation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
            && self.operation.load(Ordering::SeqCst) == operation
    }

    fn refresh(self: &Arc<Self>) {
        let (generation, operation) = self.begin_operation();
        {
            let mut state = lock(&self.state);
            state.loading = true;
            state.saving = None;
            state.problem = None;
            state.model_problem = None;
        }
        self.publish();
        let link = self.clone();
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            let result = match link.plugins.api() {
                Ok(api) => api::get(&api).await,
                Err(problem) => Err(problem),
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            match result {
                Ok(onboarding) => link.received(generation, operation, onboarding).await,
                Err(problem) => link.finish_error(generation, operation, problem),
            }
        });
    }

    async fn received(&self, generation: u64, operation: u64, onboarding: Onboarding) {
        {
            let mut state = lock(&self.state);
            state.onboarding = Some(onboarding.clone());
            state.revision += 1;
            state.providers = None;
            state.saving = None;
            state.problem = None;
            state.model_problem = None;
        }
        self.publish();
        if onboarding.step == OnboardingStep::Model {
            self.fetch_model(generation, operation, Direction::Forward)
                .await;
        } else {
            lock(&self.state).loading = false;
            self.publish();
        }
        self.maybe_mark_prompt_used();
    }

    async fn fetch_model(&self, generation: u64, operation: u64, direction: Direction) {
        {
            let mut state = lock(&self.state);
            state.providers = None;
        }
        self.publish();
        let result = match self.plugins.api() {
            Ok(api) => api::providers(&api).await,
            Err(problem) => Err(provider_problem(problem)),
        };
        if !self.operation_is_current(generation, operation) {
            return;
        }
        match result {
            Ok(providers) => {
                let configured = providers
                    .providers
                    .as_ref()
                    .is_some_and(|items| items.iter().any(|item| item.configured));
                lock(&self.state).providers = Some(providers);
                self.publish();
                if configured {
                    let step = match direction {
                        Direction::Forward => OnboardingStep::Apps,
                        Direction::Backward => OnboardingStep::Agent,
                    };
                    self.put_step(generation, operation, step, None).await;
                } else {
                    lock(&self.state).loading = false;
                    self.publish();
                }
            }
            Err(problem) => {
                let mut state = lock(&self.state);
                state.loading = false;
                state.model_problem = Some(problem);
                drop(state);
                self.publish();
            }
        }
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

    fn choose_agent(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        let onboarding = state
            .onboarding
            .as_ref()
            .ok_or_else(|| invalid("还没有取到引导状态"))?;
        let agent = onboarding
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .ok_or_else(|| invalid("这个 Agent 不在可选列表里"))?;
        if !view::installable(agent) {
            return Err(invalid("这个 Agent 不能安装"));
        }
        state.selected_agent = Some(id.to_owned());
        drop(state);
        self.publish();
        Ok(())
    }

    fn toggle_need(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        let onboarding = state
            .onboarding
            .as_ref()
            .ok_or_else(|| invalid("还没有取到引导状态"))?;
        if !onboarding.needs.iter().any(|need| need.id == id) {
            return Err(invalid("这个需求不在推荐列表里"));
        }
        if !state.selected_needs.insert(id.to_owned()) {
            state.selected_needs.remove(id);
        }
        drop(state);
        self.publish();
        Ok(())
    }

    fn continue_step(self: &Arc<Self>, skip: bool) -> Result<(), CoreError> {
        let update = {
            let state = lock(&self.state);
            let onboarding = state
                .onboarding
                .as_ref()
                .ok_or_else(|| invalid("还没有取到引导状态"))?;
            let next = next_step(onboarding.step)?;
            if onboarding.step == OnboardingStep::Agent && !skip {
                let id = selected_agent(&state, onboarding)
                    .ok_or_else(|| invalid("没有可选的 Agent"))?;
                let agent = onboarding
                    .agents
                    .iter()
                    .find(|agent| agent.id == id)
                    .ok_or_else(|| invalid("选中的 Agent 不存在"))?;
                if !view::installable(agent) {
                    return Err(invalid("选中的 Agent 不能安装"));
                }
                OnboardingUpdate {
                    step: Some(next),
                    primary_agent: Some(id),
                    ..OnboardingUpdate::default()
                }
            } else {
                OnboardingUpdate {
                    step: Some(next),
                    ..OnboardingUpdate::default()
                }
            }
        };
        self.spawn_update(update, false, Direction::Forward);
        Ok(())
    }

    fn back_step(self: &Arc<Self>) -> Result<(), CoreError> {
        let step = {
            let state = lock(&self.state);
            if state.loading || state.saving.is_some() {
                return Err(invalid("正在保存或读取引导状态"));
            }
            match state.onboarding.as_ref().map(|onboarding| onboarding.step) {
                Some(OnboardingStep::Agent) => OnboardingStep::Profile,
                Some(OnboardingStep::Model) => OnboardingStep::Agent,
                Some(OnboardingStep::Apps) => OnboardingStep::Model,
                _ => return Err(invalid("这一步不能返回")),
            }
        };
        self.spawn_update(
            OnboardingUpdate {
                step: Some(step),
                ..OnboardingUpdate::default()
            },
            false,
            Direction::Backward,
        );
        Ok(())
    }

    fn restart(self: &Arc<Self>) -> Result<(), CoreError> {
        if lock(&self.state).onboarding.is_none() {
            return Err(invalid("还没有取到引导状态"));
        }
        self.spawn_update(
            OnboardingUpdate {
                step: Some(OnboardingStep::Profile),
                ..OnboardingUpdate::default()
            },
            true,
            Direction::Forward,
        );
        Ok(())
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
        self.spawn_update(
            OnboardingUpdate {
                primary_agent: Some(id.to_owned()),
                ..OnboardingUpdate::default()
            },
            false,
            Direction::Forward,
        );
        Ok(())
    }

    fn dismiss_replaced(self: &Arc<Self>) -> Result<(), CoreError> {
        if lock(&self.state).onboarding.is_none() {
            return Err(invalid("还没有取到引导状态"));
        }
        self.spawn_update(
            OnboardingUpdate {
                replaced_agent_asked: Some(true),
                ..OnboardingUpdate::default()
            },
            false,
            Direction::Forward,
        );
        Ok(())
    }

    fn dismiss_tip(&self, tip: OnboardingTip) -> Result<(), CoreError> {
        self.storage.dismiss_tip(tip)?;
        // 写成功说明本机记录已经完整，启动时“读不了记录”的问题不再成立。
        lock(&self.state).problem = None;
        self.publish();
        Ok(())
    }

    fn spawn_update(
        self: &Arc<Self>,
        update: OnboardingUpdate,
        reset_local: bool,
        direction: Direction,
    ) {
        let (generation, operation) = self.begin_operation();
        {
            let mut state = lock(&self.state);
            state.loading = true;
            state.problem = None;
        }
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
            match result {
                Ok(onboarding) => {
                    let fetch_model = onboarding.step == OnboardingStep::Model;
                    {
                        let mut state = lock(&link.state);
                        state.onboarding = Some(onboarding);
                        state.revision += 1;
                        state.providers = None;
                        state.loading = fetch_model;
                        state.saving = None;
                        state.problem = None;
                        state.model_problem = None;
                        if reset_local {
                            state.selected_agent = None;
                            state.selected_needs.clear();
                        }
                    }
                    link.publish();
                    if fetch_model {
                        link.fetch_model(generation, operation, direction).await;
                    }
                }
                Err(problem) => {
                    let mut state = lock(&link.state);
                    state.loading = false;
                    state.problem = Some(problem);
                    drop(state);
                    link.publish();
                }
            }
        });
    }

    fn save_provider(
        self: &Arc<Self>,
        id: &str,
        key: wonton_protocol::SecretString,
    ) -> Result<(), CoreError> {
        // 粘贴常带换行，核心会当成 key 的内容拒绝；发出去之前去掉首尾空白。
        let trimmed = key.expose().trim();
        if trimmed.is_empty() {
            return Err(invalid("key 不能为空"));
        }
        let key = wonton_protocol::SecretString::new(trimmed.to_owned());
        {
            let mut state = lock(&self.state);
            if state.saving.is_some() {
                return Err(invalid("已经有一个 key 在保存"));
            }
            let onboarding = state
                .onboarding
                .as_ref()
                .ok_or_else(|| invalid("还没有取到引导状态"))?;
            if onboarding.step != OnboardingStep::Model {
                return Err(invalid("还没到接模型这一步"));
            }
            let providers = state
                .providers
                .as_ref()
                .and_then(|providers| providers.providers.as_ref())
                .ok_or_else(|| invalid("模型网关没有声明服务商列表"))?;
            if !providers.iter().any(|provider| provider.id == id) {
                return Err(invalid("这个服务商不在列表里"));
            }
            state.saving = Some(id.to_owned());
            state.model_problem = None;
            state.problem = None;
        }
        let (generation, operation) = self.begin_operation();
        self.publish();
        let link = self.clone();
        let id = id.to_owned();
        self.runtime.spawn(async move {
            let result = match link.plugins.api() {
                Ok(api) => api::save_provider(&api, &id, ModelProviderKey { key }).await,
                Err(problem) => Err(provider_problem(problem)),
            };
            link.finish_provider_save(generation, operation, result)
                .await;
        });
        Ok(())
    }

    async fn finish_provider_save(
        &self,
        generation: u64,
        operation: u64,
        result: Result<(), ModelProviderProblem>,
    ) {
        if !self.operation_is_current(generation, operation) {
            return;
        }
        match result {
            Ok(()) => {
                let providers = match self.plugins.api() {
                    Ok(api) => api::providers(&api).await,
                    Err(problem) => Err(provider_problem(problem)),
                };
                if !self.operation_is_current(generation, operation) {
                    return;
                }
                if let Ok(providers) = &providers {
                    lock(&self.state).providers = Some(providers.clone());
                }
                self.put_step(generation, operation, OnboardingStep::Apps, providers.err())
                    .await;
            }
            Err(problem) => {
                if problem.reason == ModelProviderFailure::ProviderUnreachable
                    && let Ok(api) = self.plugins.api()
                    && let Ok(providers) = api::providers(&api).await
                    && self.operation_is_current(generation, operation)
                {
                    lock(&self.state).providers = Some(providers);
                }
                if !self.operation_is_current(generation, operation) {
                    return;
                }
                let mut state = lock(&self.state);
                if problem.reason == ModelProviderFailure::ProviderUnreachable {
                    // 退出码 4 保证 key 已保存，即使随后取服务商列表失败也能继续。
                    let LocalState {
                        providers, saving, ..
                    } = &mut *state;
                    if let Some(items) = providers
                        .as_mut()
                        .and_then(|providers| providers.providers.as_mut())
                        && let Some(provider) = items
                            .iter_mut()
                            .find(|provider| Some(provider.id.as_str()) == saving.as_deref())
                    {
                        provider.configured = true;
                    }
                }
                state.saving = None;
                state.model_problem = Some(problem);
                drop(state);
                self.publish();
            }
        }
    }

    async fn put_step(
        &self,
        generation: u64,
        operation: u64,
        step: OnboardingStep,
        follow_up_problem: Option<ModelProviderProblem>,
    ) {
        let result = match self.plugins.api() {
            Ok(api) => {
                api::update(
                    &api,
                    &OnboardingUpdate {
                        step: Some(step),
                        ..OnboardingUpdate::default()
                    },
                )
                .await
            }
            Err(problem) => Err(problem),
        };
        if !self.operation_is_current(generation, operation) {
            return;
        }
        let mut state = lock(&self.state);
        state.loading = false;
        state.saving = None;
        match result {
            Ok(onboarding) => {
                state.onboarding = Some(onboarding);
                state.revision += 1;
                state.problem = None;
                state.model_problem = follow_up_problem;
            }
            Err(problem) => state.problem = Some(problem),
        }
        drop(state);
        self.publish();
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

    fn mark_prompt_used(&self) {
        if let Err(error) = self.storage.mark_prompts_used() {
            lock(&self.state).problem = Some(storage_problem(error));
        }
        self.publish();
    }

    fn maybe_mark_prompt_used(&self) {
        if !self.storage.snapshot().prompts_used && self.prompt_ready() {
            self.mark_prompt_used();
        }
    }

    fn prompt_ready(&self) -> bool {
        let state = lock(&self.state);
        let Some(onboarding) = state.onboarding.as_ref() else {
            return false;
        };
        let Some(agent) = onboarding
            .agents
            .iter()
            .find(|agent| Some(agent.id.as_str()) == onboarding.primary_agent.as_deref())
        else {
            return false;
        };
        let Some(bot) = agent.bot_user_id.as_deref() else {
            return false;
        };
        let sent_rooms = lock(&self.sent_rooms);
        self.chat.view().rooms.iter().any(|room| {
            sent_rooms.contains(&room.room_id) && room.direct_user_id.as_deref() == Some(bot)
        })
    }

    fn publish(&self) {
        let state = lock(&self.state);
        let onboarding = state.onboarding.clone();
        let language = text::language(&lock(&self.languages));
        let records = self.storage.snapshot();
        let chat = self.chat.view();
        let sent_rooms = lock(&self.sent_rooms).clone();
        let (view, primary_bot) = render(
            &state,
            onboarding.as_ref(),
            &records,
            &chat,
            &sent_rooms,
            language,
        );
        drop(state);
        self.chat.set_primary_bot(primary_bot);
        self.view.send_replace(view);
    }
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
        link.maybe_mark_prompt_used();
    }
}

async fn follow_sent(link: Weak<OnboardingLink>, mut sent: Receiver<Vec<String>>) {
    while sent.changed().await.is_ok() {
        let sent_rooms = sent.borrow_and_update().clone();
        let Some(link) = link.upgrade() else { return };
        lock(&link.sent_rooms).extend(sent_rooms.iter().cloned());
        for room_id in sent_rooms {
            if let Err(error) = link.storage.record_prompt_room(&room_id) {
                lock(&link.state).problem = Some(storage_problem(error));
            }
        }
        link.publish();
        link.maybe_mark_prompt_used();
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
