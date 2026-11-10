//! 最后一页（ADR-0038）：列出所有选择和要装的插件的权限；点完成后依次保存资料、模型和“走完了”，再在后台装 Agent 和
//! 应用。点完成之前这里只取安装预览，不改任何东西。

use std::collections::HashSet;
use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, InstallPreview, ModelProviderFailure, OnboardingCommit, OnboardingItemProblem,
    OnboardingItemState, OnboardingReviewDetail, OnboardingReviewItem, OnboardingReviewKind,
    OnboardingStep, OnboardingUpdate, PluginFailure, PluginIcon, PluginProblem, PluginTask,
    PluginTaskState, normalized_email,
};

use super::draft::ModelDraft;
use super::{LocalState, OnboardingLink, api, invalid, lock};
use crate::l10n::{CATALOG, language};

pub(super) const PROFILE: &str = "profile";
pub(super) const MODEL: &str = "model";

pub(super) struct ReviewEntry {
    pub id: String,
    pub kind: OnboardingReviewKind,
    pub titles: Vec<String>,
    pub detail: Option<OnboardingReviewDetail>,
    pub icon: Option<PluginIcon>,
    /// 要装时的安装来源。
    pub source: Option<String>,
    pub preview: Option<InstallPreview>,
    pub state: OnboardingItemState,
}

impl ReviewEntry {
    fn new(id: &str, kind: OnboardingReviewKind, titles: Vec<String>) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            titles,
            detail: None,
            icon: None,
            source: None,
            preview: None,
            state: OnboardingItemState::Ready,
        }
    }

    pub fn item(&self) -> OnboardingReviewItem {
        OnboardingReviewItem {
            id: self.id.clone(),
            kind: self.kind,
            titles: self.titles.clone(),
            detail: self.detail.clone(),
            icon: self.icon.clone(),
            permissions: self
                .preview
                .as_ref()
                .map_or_else(Vec::new, |preview| preview.permissions.clone()),
            notices: self
                .preview
                .as_ref()
                .map_or_else(Vec::new, |preview| preview.notices.clone()),
            state: self.state.clone(),
        }
    }

    fn installs(&self) -> bool {
        self.source.is_some()
    }
}

/// 按记下的选择列出这一页的项。要装的插件先是 `Preparing`，取到预览后再定。
pub(super) fn entries(state: &LocalState) -> Vec<ReviewEntry> {
    let Some(onboarding) = state.onboarding.as_ref() else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    // 服务器太旧没显示资料这一步时不列它：它存不了。
    if !state.draft.profile_skipped && !state.unavailable.contains(&OnboardingStep::Profile) {
        let (name, email) = state.draft.profile_values(state.owner.as_ref());
        let mut entry = ReviewEntry::new(
            PROFILE,
            OnboardingReviewKind::Profile,
            vec![name.trim().to_owned()],
        );
        entry.detail = normalized_email(Some(&email)).map(|email| OnboardingReviewDetail::Email {
            email: email.to_owned(),
        });
        entries.push(entry);
    }
    let agent_id = state.draft.agent(onboarding);
    if let Some(agent) = agent_id
        .as_deref()
        .and_then(|id| onboarding.agents.iter().find(|agent| agent.id == id))
    {
        let mut entry = ReviewEntry::new(
            &agent.id,
            OnboardingReviewKind::Agent,
            vec![agent.title.clone()],
        );
        if agent.status.is_some() {
            entry.detail = (onboarding.primary_agent.as_deref() != Some(agent.id.as_str()))
                .then_some(OnboardingReviewDetail::SetAsDefault);
        } else {
            prepare(&mut entry, agent.source.clone(), agent.blocked.clone());
        }
        entries.push(entry);
    }
    if let Some(model) = state.draft.model(state.providers.as_ref()) {
        entries.push(model_entry(model, state));
    }
    let mut seen: HashSet<&str> = agent_id.iter().map(String::as_str).collect();
    let needs = state.draft.needs(onboarding);
    for plugin in onboarding
        .needs
        .iter()
        .filter(|need| needs.contains(&need.id))
        .flat_map(|need| &need.plugins)
    {
        if plugin.installed_version.is_some() || !seen.insert(&plugin.id) {
            continue;
        }
        let mut entry = ReviewEntry::new(
            &plugin.id,
            OnboardingReviewKind::App,
            vec![plugin.title.clone()],
        );
        entry.icon = plugin.icon.clone();
        prepare(
            &mut entry,
            Some(plugin.source.clone()),
            plugin.blocked.clone(),
        );
        entries.push(entry);
    }
    entries
}

fn prepare(entry: &mut ReviewEntry, source: Option<String>, blocked: Option<String>) {
    entry.state = match (&source, blocked) {
        (_, Some(message)) => OnboardingItemState::Blocked {
            message: Some(message),
        },
        (Some(_), None) => OnboardingItemState::Preparing,
        (None, None) => OnboardingItemState::Blocked { message: None },
    };
    entry.source = source;
}

fn model_entry(model: &ModelDraft, state: &LocalState) -> ReviewEntry {
    let (titles, detail) = match model {
        ModelDraft::Current => {
            let titles = state
                .providers
                .iter()
                .flat_map(|providers| &providers.configured)
                .map(|provider| provider.title.clone())
                .collect();
            (titles, Some(OnboardingReviewDetail::KeepCurrent))
        }
        ModelDraft::Tested {
            input,
            title,
            reachable,
        } => {
            let model = match input {
                wonton_protocol::ModelProviderInput::Custom { model, .. } => Some(model.clone()),
                wonton_protocol::ModelProviderInput::Preset { .. } => None,
            };
            let detail = if *reachable {
                model.map(|model| OnboardingReviewDetail::Model { model })
            } else {
                Some(OnboardingReviewDetail::Unreachable { model })
            };
            (vec![title.clone()], detail)
        }
        ModelDraft::Login { title, .. } => (
            vec![title.clone()],
            Some(OnboardingReviewDetail::SignInAfterSetup),
        ),
        ModelDraft::Skipped => (Vec::new(), None),
    };
    let mut entry = ReviewEntry::new(MODEL, OnboardingReviewKind::Model, titles);
    entry.detail = detail;
    entry
}

/// 取到预览后定下这一项：装不了的、要先填资料而资料页没填邮箱的，完成不能点。
pub(super) fn apply_preview(entry: &mut ReviewEntry, preview: InstallPreview, email_filled: bool) {
    entry.icon = preview.icon.clone().or_else(|| entry.icon.take());
    entry.state = match (&preview.blocked, &preview.missing_profile) {
        // 安装进展按插件 ID 对到这一项上，对不上就看不到进展。
        _ if preview.id != entry.id => OnboardingItemState::Blocked {
            message: Some(l10n!(
                CATALOG,
                language(),
                "onboarding-review-source-mismatch",
                installed = preview.id.as_str(),
                listed = entry.id.as_str()
            )),
        },
        (Some(message), _) => OnboardingItemState::Blocked {
            message: Some(message.clone()),
        },
        (None, Some(message)) if !email_filled => OnboardingItemState::Blocked {
            message: Some(message.clone()),
        },
        _ => OnboardingItemState::Ready,
    };
    entry.preview = Some(preview);
}

pub(super) fn can_commit(entries: &[ReviewEntry]) -> bool {
    entries.iter().all(|entry| {
        matches!(
            entry.state,
            OnboardingItemState::Ready | OnboardingItemState::Done
        )
    })
}

/// 插件任务的进展落到对应的项上。只看点了完成以后发起的安装。
pub(super) fn follow_tasks(entries: &mut [ReviewEntry], tasks: &[PluginTask]) -> bool {
    let mut changed = false;
    for entry in entries.iter_mut().filter(|entry| entry.installs()) {
        // 失败和不确定都不是这一项在这里的终点：重试以后上一份快照还带着旧的那次失败，不能让它把刚起来的
        // 安装卡住。
        if !matches!(
            entry.state,
            OnboardingItemState::Waiting
                | OnboardingItemState::Running
                | OnboardingItemState::Failed { .. }
                | OnboardingItemState::Uncertain { .. }
        ) {
            continue;
        }
        let Some(task) = tasks.iter().find(|task| task.id == entry.id) else {
            continue;
        };
        tracing::debug!(id = entry.id.as_str(), ?entry.state, ?task.state, "followed a task");
        let next = match &task.state {
            PluginTaskState::Running => OnboardingItemState::Running,
            PluginTaskState::Succeeded => OnboardingItemState::Done,
            PluginTaskState::Failed { reason, message } => OnboardingItemState::Failed {
                problem: OnboardingItemProblem::Plugin {
                    problem: PluginProblem {
                        reason: *reason,
                        min_version: None,
                        message: message.clone(),
                    },
                },
            },
            PluginTaskState::Uncertain { message } => OnboardingItemState::Uncertain {
                message: message.clone(),
            },
        };
        if entry.state != next {
            entry.state = next;
            changed = true;
        }
    }
    changed
}

fn plugin_failed(problem: PluginProblem) -> OnboardingItemState {
    OnboardingItemState::Failed {
        problem: OnboardingItemProblem::Plugin { problem },
    }
}

fn core_problem(error: &CoreError) -> PluginProblem {
    let reason = match error {
        CoreError::InvalidInput { .. } => PluginFailure::Invalid,
        CoreError::TransportUnavailable { .. } => PluginFailure::TransportUnavailable,
        CoreError::Unauthorized { .. } => PluginFailure::Unauthorized,
        CoreError::Storage { .. } | CoreError::NotReady { .. } | CoreError::Upstream { .. } => {
            PluginFailure::Upstream
        }
        CoreError::ServerTooOld { .. } => PluginFailure::ServerTooOld,
    };
    PluginProblem {
        reason,
        message: error.to_string(),
        min_version: None,
    }
}

impl OnboardingLink {
    /// 进最后一页：列出项，给要装的取预览。
    pub(super) fn enter_review(self: &Arc<Self>) {
        let (generation, review, sources) = {
            let mut state = lock(&self.state);
            state.review = entries(&state);
            state.review_generation += 1;
            state.commit = OnboardingCommit::NotStarted;
            state.step = Some(OnboardingStep::Review);
            let sources: Vec<(String, String)> = state
                .review
                .iter()
                .filter(|entry| entry.state == OnboardingItemState::Preparing)
                .filter_map(|entry| Some((entry.id.clone(), entry.source.clone()?)))
                .collect();
            (self.generation(), state.review_generation, sources)
        };
        self.publish();
        for (id, source) in sources {
            self.spawn_preview(generation, review, id, source, false);
        }
    }

    /// `install` 为真时预览完接着装：点了完成以后的重试和重取预览。
    fn spawn_preview(
        self: &Arc<Self>,
        generation: u64,
        review: u64,
        id: String,
        source: String,
        install: bool,
    ) {
        let link = self.clone();
        self.runtime.spawn(async move {
            let result = match link.plugins.api() {
                Ok(api) => api::preview(&api, &source).await,
                Err(problem) => Err(problem),
            };
            let preview = {
                let mut state = lock(&link.state);
                if link.generation() != generation || state.review_generation != review {
                    return;
                }
                let (_, email) = state.draft.profile_values(state.owner.as_ref());
                let email_filled = !email.trim().is_empty();
                let Some(entry) = state.review.iter_mut().find(|entry| entry.id == id) else {
                    return;
                };
                match result {
                    Ok(preview) => {
                        let confirmed = entry.preview.take().map(|old| old.permissions);
                        apply_preview(entry, preview, email_filled);
                        let preview = entry.preview.as_ref();
                        let changed = confirmed.is_some_and(|old| {
                            preview.is_some_and(|preview| preview.permissions != old)
                        });
                        if install && changed {
                            // 点完成确认的是列出的那些权限，变了就不能替主人装。
                            entry.state = OnboardingItemState::Blocked {
                                message: Some(l10n!(
                                    CATALOG,
                                    language(),
                                    "onboarding-review-permissions-changed"
                                )),
                            };
                            None
                        } else if install && entry.state == OnboardingItemState::Ready {
                            entry.state = OnboardingItemState::Waiting;
                            entry.preview.clone()
                        } else {
                            None
                        }
                    }
                    Err(problem) => {
                        entry.state = plugin_failed(problem);
                        None
                    }
                }
            };
            if let Some(preview) = preview {
                link.install(&id, preview);
            }
            link.publish();
        });
    }

    fn install(self: &Arc<Self>, id: &str, preview: InstallPreview) {
        if let Err(error) = self.plugins.install(preview, Default::default()) {
            let mut state = lock(&self.state);
            if let Some(entry) = state.review.iter_mut().find(|entry| entry.id == id) {
                entry.state = plugin_failed(core_problem(&error));
            }
        }
    }

    /// 点完成，或者重试停下来的资料、模型：从没做完的那一项接着做。
    pub(super) fn commit(self: &Arc<Self>) -> Result<(), CoreError> {
        {
            let mut state = lock(&self.state);
            if state.commit != OnboardingCommit::NotStarted || !can_commit(&state.review) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-review-items-not-ready"
                )));
            }
            state.commit = OnboardingCommit::Running;
            state.problem = None;
        }
        self.publish();
        let link = self.clone();
        let generation = self.generation();
        self.runtime.spawn(async move {
            let saved = link.save_profile(generation).await
                && link.save_model(generation).await
                && link.finish(generation).await;
            if !saved {
                if link.generation() == generation {
                    lock(&link.state).commit = OnboardingCommit::NotStarted;
                    link.publish();
                }
                return;
            }
            link.start_installs(generation);
        });
        Ok(())
    }

    fn set_entry(&self, id: &str, next: OnboardingItemState) {
        let mut state = lock(&self.state);
        if let Some(entry) = state.review.iter_mut().find(|entry| entry.id == id) {
            entry.state = next;
        }
        drop(state);
        self.publish();
    }

    fn pending(&self, id: &str) -> bool {
        lock(&self.state)
            .review
            .iter()
            .any(|entry| entry.id == id && entry.state != OnboardingItemState::Done)
    }

    async fn save_profile(&self, generation: u64) -> bool {
        if !self.pending(PROFILE) {
            return true;
        }
        // 服务器太旧，资料这一步整步没显示：不存，也不让完成卡在这里。
        if lock(&self.state)
            .unavailable
            .contains(&OnboardingStep::Profile)
        {
            self.set_entry(PROFILE, OnboardingItemState::Done);
            return true;
        }
        let update = {
            let state = lock(&self.state);
            state.draft.profile_update(state.owner.as_ref())
        };
        let Some(update) = update else {
            self.set_entry(PROFILE, OnboardingItemState::Done);
            return true;
        };
        self.set_entry(PROFILE, OnboardingItemState::Running);
        let result = match self.plugins.api() {
            Ok(api) => api::save_me(&api, &update).await,
            Err(problem) => Err(problem),
        };
        if self.generation() != generation {
            return false;
        }
        match result {
            Ok(saved) => {
                if let Some(name) = saved.display_name.clone() {
                    self.chat.set_owner_display_name(name);
                }
                {
                    let mut state = lock(&self.state);
                    state.owner = Some(saved);
                    state.draft.profile = None;
                }
                self.set_entry(PROFILE, OnboardingItemState::Done);
                true
            }
            Err(problem) => {
                self.set_entry(PROFILE, plugin_failed(problem));
                false
            }
        }
    }

    async fn save_model(&self, generation: u64) -> bool {
        if !self.pending(MODEL) {
            return true;
        }
        let input = {
            let state = lock(&self.state);
            match state.draft.model(state.providers.as_ref()) {
                Some(ModelDraft::Tested { input, .. }) => Some(input.clone()),
                _ => None,
            }
        };
        let Some(input) = input else {
            self.set_entry(MODEL, OnboardingItemState::Done);
            return true;
        };
        self.set_entry(MODEL, OnboardingItemState::Running);
        let result = match self.plugins.api() {
            Ok(api) => api::save_provider(&api, input).await,
            Err(problem) => Err(super::provider_problem(problem)),
        };
        if self.generation() != generation {
            return false;
        }
        match result {
            // 连不上服务商时 key 已经保存（ADR-0034），测试时已经告诉过主人。
            Ok(()) => {}
            Err(problem) if problem.reason == ModelProviderFailure::ProviderUnreachable => {}
            Err(problem) => {
                let failed = OnboardingItemState::Failed {
                    problem: OnboardingItemProblem::Model { problem },
                };
                self.set_entry(MODEL, failed);
                return false;
            }
        }
        lock(&self.state).draft.model = None;
        self.set_entry(MODEL, OnboardingItemState::Done);
        true
    }

    async fn finish(&self, generation: u64) -> bool {
        let update = {
            let state = lock(&self.state);
            let Some(onboarding) = state.onboarding.as_ref() else {
                return false;
            };
            OnboardingUpdate {
                step: Some(OnboardingStep::Finished),
                primary_agent: state.draft.agent(onboarding),
                replaced_agent_asked: None,
            }
        };
        let result = match self.plugins.api() {
            Ok(api) => api::update(&api, &update).await,
            Err(problem) => Err(problem),
        };
        if self.generation() != generation {
            return false;
        }
        let mut state = lock(&self.state);
        match result {
            Ok(onboarding) => {
                state.onboarding = Some(onboarding);
                state.revision += 1;
                true
            }
            Err(problem) => {
                state.problem = Some(problem);
                false
            }
        }
    }

    fn start_installs(self: &Arc<Self>, generation: u64) {
        let mut installs = Vec::new();
        let mut previews = Vec::new();
        let review = {
            let mut state = lock(&self.state);
            if self.generation() != generation {
                return;
            }
            state.commit = OnboardingCommit::Done;
            for entry in &mut state.review {
                if entry.state == OnboardingItemState::Done {
                    continue;
                }
                // 不用装的（已经装着的 Agent）随“走完了”一起保存好了。
                let (Some(preview), Some(source)) = (entry.preview.clone(), entry.source.clone())
                else {
                    entry.state = OnboardingItemState::Done;
                    continue;
                };
                // 要邮箱的预览是在保存资料之前取的，按它装会被拒，重取一次。取到之前不是 Waiting：
                // 同一个插件以前的任务还在列表里，不能把它的结果当成这次的。
                if preview.missing_profile.is_some() {
                    entry.state = OnboardingItemState::Preparing;
                    previews.push((entry.id.clone(), source));
                } else {
                    entry.state = OnboardingItemState::Waiting;
                    installs.push((entry.id.clone(), preview));
                }
            }
            state.review_generation
        };
        // 都交给家里的服务器：它单独执行安装，App 关掉也照样装完（ADR-0017）。
        for (id, preview) in installs {
            self.install(&id, preview);
        }
        for (id, source) in previews {
            self.spawn_preview(generation, review, id, source, true);
        }
        let tasks = self.plugins.view().tasks;
        follow_tasks(&mut lock(&self.state).review, &tasks);
        self.publish();
    }

    pub(super) fn retry_item(self: &Arc<Self>, id: &str) -> Result<(), CoreError> {
        let (commit, source, failed) = {
            let state = lock(&self.state);
            let entry = state
                .review
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| {
                    invalid(&l10n!(
                        CATALOG,
                        language(),
                        "onboarding-review-item-missing"
                    ))
                })?;
            (
                state.commit,
                entry.source.clone(),
                matches!(entry.state, OnboardingItemState::Failed { .. }),
            )
        };
        if !failed || commit == OnboardingCommit::Running {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-review-no-failure"
            )));
        }
        let Some(source) = source else {
            // 资料和模型：点完成之后停在它们上，从它们接着做。
            self.set_entry(id, OnboardingItemState::Ready);
            return self.commit();
        };
        let review = {
            let mut state = lock(&self.state);
            let entry = state.review.iter_mut().find(|entry| entry.id == id);
            if let Some(entry) = entry {
                entry.state = OnboardingItemState::Preparing;
                entry.preview = None;
            }
            state.review_generation
        };
        self.publish();
        let install = commit == OnboardingCommit::Done;
        self.spawn_preview(self.generation(), review, id.to_owned(), source, install);
        Ok(())
    }
}
