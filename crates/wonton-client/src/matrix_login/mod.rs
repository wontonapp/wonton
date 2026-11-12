//! 用现成的 Matrix 账号登录（ADR-0051）：一次只走一个流程，进展发布在服务器列表的 `matrix_login` 上。
//!
//! 每个流程有自己的 tokio 运行时和一个新占的槽位号（[`ServerList::reserve_login_slot`]）：登录用的客户端把存储开在
//! 那个槽位的目录里，设备密钥就在之后会话要用的地方。流程结束（成功、失败、取消、被新流程顶替）都先停掉它的运行时，
//! 登录用的客户端连同 PKCE verifier、state 一起丢掉；不成功的把占的目录删掉，原来的槽位不动。成功时才把槽位加进列表并
//! 切过去（[`SlotSwitch`]）。
//!
//! 发现、登录和换令牌都在 wonton-matrix 里用 matrix-sdk 的现成接口做，这里只管状态、校验和收尾。

mod tasks;

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use url::Url;
use wonton_l10n::l10n;
use wonton_matrix::{OwnedUserId, PendingLogin};
use wonton_protocol::{
    Action, CoreError, MatrixAccount, MatrixLoginFailure, MatrixLoginMethod, MatrixLoginState,
    MatrixLoginViewModel, MatrixOidcCallback, MatrixOidcLogin, MatrixPasswordLogin,
};

use crate::l10n::{CATALOG, language};
use crate::scope::ScopeRuntime;
use crate::servers::ServerList;

/// 不填服务器时用的。
const DEFAULT_HOMESERVER: &str = "matrix.org";

/// 核心接受的 OIDC 回调地址，经 `MatrixLoginViewModel.oidc_redirect_uri` 给壳；壳交来的不是它就拒绝。
///
/// matrix.org 的授权服务器（MAS）动态注册时要求原生 App 的回调 scheme 是 `client_uri` 域名倒过来写
/// （[`tasks::CLIENT_URI`]），`wonton://` 会被拒成 `invalid_redirect_uri`。只有一个斜杠是 RFC 8252 7.1 的私有 scheme 写法。
const OIDC_REDIRECT_URI: &str = "io.github.wontonapp:/matrix-login";

/// 授权地址发出去以后，回调最晚多久内回来。授权服务器的授权码一般只活几分钟，这里给够在浏览器里登录的时间。
const OIDC_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// 登录成功后把槽位加进列表并切过去；由 [`ClientCore`](crate::ClientCore) 实现，它管着作用域。
pub(crate) trait SlotSwitch: Send + Sync {
    fn add_matrix_slot(&self, slot: &str, account: MatrixAccount) -> Result<(), CoreError>;
}

pub(crate) struct MatrixLogins {
    servers: Arc<ServerList>,
    switch: OnceLock<Weak<dyn SlotSwitch>>,
    /// 当前的流程。异步的步骤回来时先看自己还是不是它，不是就作罢：新流程不吃旧流程的结果。
    current: Mutex<Option<Arc<Flow>>>,
}

struct Flow {
    id: String,
    /// 占着的槽位号，登录的存储在它的目录里。
    slot: String,
    runtime: ScopeRuntime,
    state: Mutex<FlowState>,
}

struct FlowState {
    view: MatrixLoginViewModel,
    /// 发现完成后才有；流程结束时丢掉，登录用的客户端跟着释放。
    pending: Option<Arc<PendingLogin>>,
    oidc: Option<Oidc>,
}

/// 等着浏览器回调的 OIDC 授权。`state` 和授权地址一样是秘密，不进日志。
struct Oidc {
    redirect: Url,
    state: String,
    deadline: Instant,
    device_name: String,
}

impl MatrixLogins {
    pub(crate) fn new(servers: Arc<ServerList>) -> Arc<Self> {
        Arc::new(Self {
            servers,
            switch: OnceLock::new(),
            current: Mutex::new(None),
        })
    }

    /// 核心建好以后接上；只接一次。
    pub(crate) fn bind<S: SlotSwitch + 'static>(&self, switch: Weak<S>) {
        let switch: Weak<dyn SlotSwitch> = switch;
        let _ = self.switch.set(switch);
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::StartMatrixLogin { homeserver } => self.start(homeserver),
            Action::SubmitMatrixPassword { login } => self.submit_password(login),
            Action::StartMatrixOidc { login } => self.start_oidc(login),
            Action::CompleteMatrixOidc { callback } => self.complete_oidc(&callback),
            Action::CancelMatrixLogin { login_id } => self.cancel(&login_id),
            _ => Err(invalid("servers-invalid-action")),
        }
    }

    /// 开始一个新流程；有旧的就先结束它（壳在失败页上点重试直接发这个）。
    fn start(self: &Arc<Self>, homeserver: Option<String>) -> Result<(), CoreError> {
        let mut current = self.lock();
        if let Some(old) = current.take() {
            self.end(&old);
        }
        let input =
            homeserver.map_or_else(|| DEFAULT_HOMESERVER.to_owned(), |h| h.trim().to_owned());
        let slot = self.servers.reserve_login_slot()?;
        let store = match tasks::store(&self.servers, &slot) {
            Ok(store) => store,
            Err(error) => {
                if let Err(release) = self.servers.release_login_slot(&slot) {
                    tracing::warn!(%release, "could not delete an unfinished Matrix sign-in");
                }
                return Err(error);
            }
        };
        let id = login_id();
        let view = MatrixLoginViewModel {
            login_id: id.clone(),
            homeserver: input.clone(),
            methods: Vec::new(),
            oidc_redirect_uri: OIDC_REDIRECT_URI.to_owned(),
            state: MatrixLoginState::Discovering,
        };
        let flow = Arc::new(Flow {
            id,
            slot,
            runtime: ScopeRuntime::new(),
            state: Mutex::new(FlowState {
                view: view.clone(),
                pending: None,
                oidc: None,
            }),
        });
        *current = Some(flow.clone());
        self.servers.set_login(Some(view));
        drop(current);
        let logins = self.clone();
        flow.runtime
            .spawn(tasks::discover(logins, flow.clone(), input, store));
        Ok(())
    }

    fn submit_password(self: &Arc<Self>, login: MatrixPasswordLogin) -> Result<(), CoreError> {
        let device_name = device_name(&login.device_name)?;
        let user_id = OwnedUserId::try_from(login.user_id.trim())
            .map_err(|_| invalid("matrix-login-invalid-user-id"))?;
        if login.password.expose().is_empty() {
            return Err(invalid("matrix-login-password-required"));
        }
        let (flow, pending) = self.ready(&login.login_id, MatrixLoginMethod::Password, false)?;
        let logins = self.clone();
        flow.runtime.spawn(tasks::password(
            logins,
            flow.clone(),
            pending,
            user_id,
            login.password,
            device_name,
        ));
        Ok(())
    }

    fn start_oidc(self: &Arc<Self>, login: MatrixOidcLogin) -> Result<(), CoreError> {
        let device_name = device_name(&login.device_name)?;
        if login.redirect_uri.trim() != OIDC_REDIRECT_URI {
            return Err(invalid("matrix-login-redirect-not-allowed"));
        }
        let redirect = Url::parse(OIDC_REDIRECT_URI).expect("the redirect URI is valid");
        // 浏览器里放弃了的话可以再开一次授权，换新的 state。
        let (flow, pending) = self.ready(&login.login_id, MatrixLoginMethod::Oidc, true)?;
        let logins = self.clone();
        flow.runtime.spawn(tasks::oidc_url(
            logins,
            flow.clone(),
            pending,
            redirect,
            device_name,
        ));
        Ok(())
    }

    /// 核对回调属于这次授权，再去换令牌。对不上的回调让这次登录失败，不留着半截授权。
    fn complete_oidc(self: &Arc<Self>, callback: &MatrixOidcCallback) -> Result<(), CoreError> {
        let current = self.lock();
        let flow = current_flow(&current, &callback.login_id)?.clone();
        let mut state = lock(&flow.state);
        let awaiting = matches!(state.view.state, MatrixLoginState::AwaitingOidc { .. });
        let (Some(oidc), Some(pending), true) = (&state.oidc, state.pending.clone(), awaiting)
        else {
            return Err(invalid("matrix-login-wrong-step"));
        };
        let device_name = oidc.device_name.clone();
        match check_callback(oidc, callback.callback_url.expose()) {
            Ok(url) => {
                state.oidc = None;
                state.view.state = MatrixLoginState::Authenticating;
                self.servers.set_login(Some(state.view.clone()));
                drop(state);
                drop(current);
                let logins = self.clone();
                flow.runtime.spawn(tasks::finish_oidc(
                    logins,
                    flow.clone(),
                    pending,
                    url,
                    device_name,
                ));
            }
            Err(reason) => {
                drop(state);
                self.fail_locked(&current, &flow, reason);
            }
        }
        Ok(())
    }

    fn cancel(self: &Arc<Self>, login_id: &str) -> Result<(), CoreError> {
        let current = self.lock();
        let flow = current_flow(&current, login_id)?;
        let mut state = lock(&flow.state);
        if is_finished(&state.view.state) {
            return Ok(());
        }
        state.view.state = MatrixLoginState::Cancelled;
        state.pending = None;
        state.oidc = None;
        self.servers.set_login(Some(state.view.clone()));
        drop(state);
        self.end(flow);
        Ok(())
    }

    /// 流程停在“可以选方式”上、方式是发现到的，就进入“认证中”，返回登录用的客户端。
    fn ready(
        &self,
        login_id: &str,
        method: MatrixLoginMethod,
        again: bool,
    ) -> Result<(Arc<Flow>, Arc<PendingLogin>), CoreError> {
        let current = self.lock();
        let flow = current_flow(&current, login_id)?;
        let mut state = lock(&flow.state);
        let at_step = match state.view.state {
            MatrixLoginState::Ready => true,
            MatrixLoginState::AwaitingOidc { .. } => again,
            _ => false,
        };
        let pending = state.pending.clone().filter(|_| at_step);
        let Some(pending) = pending else {
            return Err(invalid("matrix-login-wrong-step"));
        };
        if !state.view.methods.contains(&method) {
            return Err(invalid("matrix-login-method-not-offered"));
        }
        state.oidc = None;
        state.view.state = MatrixLoginState::Authenticating;
        self.servers.set_login(Some(state.view.clone()));
        Ok((flow.clone(), pending))
    }

    /// 改这个流程的状态并发布；它已经不是当前的流程（被取消、被顶替）就什么都不做，返回假。
    fn update(&self, flow: &Arc<Flow>, change: impl FnOnce(&mut FlowState)) -> bool {
        let current = self.lock();
        if !current.as_ref().is_some_and(|c| Arc::ptr_eq(c, flow)) {
            return false;
        }
        let mut state = lock(&flow.state);
        if is_finished(&state.view.state) {
            return false;
        }
        change(&mut state);
        self.servers.set_login(Some(state.view.clone()));
        true
    }

    /// 流程失败：发布类别，结束流程，删掉占的目录。
    fn fail(&self, flow: &Arc<Flow>, reason: MatrixLoginFailure) {
        let current = self.lock();
        self.fail_locked(&current, flow, reason);
    }

    fn fail_locked(
        &self,
        current: &MutexGuard<'_, Option<Arc<Flow>>>,
        flow: &Arc<Flow>,
        reason: MatrixLoginFailure,
    ) {
        if !current.as_ref().is_some_and(|c| Arc::ptr_eq(c, flow)) {
            return;
        }
        let mut state = lock(&flow.state);
        if is_finished(&state.view.state) {
            return;
        }
        state.view.state = MatrixLoginState::Failed { reason };
        state.pending = None;
        state.oidc = None;
        self.servers.set_login(Some(state.view.clone()));
        drop(state);
        self.end(flow);
    }

    /// 停掉流程的运行时；没成功的等它停下再删掉占的槽位目录（登录用的存储还开着时不能删）。不阻塞调用方：
    /// 调用方可能是壳的主线程，也可能就在这个运行时里。
    fn end(&self, flow: &Arc<Flow>) {
        let release = {
            let mut state = lock(&flow.state);
            state.pending = None;
            state.oidc = None;
            if !is_finished(&state.view.state) {
                state.view.state = MatrixLoginState::Cancelled;
            }
            !matches!(state.view.state, MatrixLoginState::Succeeded { .. })
        };
        let stopping = flow.runtime.stopping();
        let servers = self.servers.clone();
        let slot = flow.slot.clone();
        std::thread::spawn(move || {
            if let Some(stopping) = stopping {
                let _ = stopping.join();
            }
            if release && let Err(error) = servers.release_login_slot(&slot) {
                tracing::warn!(%error, "could not delete an unfinished Matrix sign-in");
            }
        });
    }

    fn lock(&self) -> MutexGuard<'_, Option<Arc<Flow>>> {
        lock(&self.current)
    }
}

fn current_flow<'a>(
    current: &'a MutexGuard<'_, Option<Arc<Flow>>>,
    login_id: &str,
) -> Result<&'a Arc<Flow>, CoreError> {
    current
        .as_ref()
        .filter(|flow| flow.id == login_id)
        .ok_or_else(|| invalid("matrix-login-not-current"))
}

/// 回调要发往注册的地址，没过期，state 是这次授权的；返回要交给 SDK 的完整回调。
fn check_callback(oidc: &Oidc, callback: &str) -> Result<Url, MatrixLoginFailure> {
    let url = Url::parse(callback.trim()).map_err(|_| MatrixLoginFailure::InvalidCallback)?;
    let same_target = url.scheme() == oidc.redirect.scheme()
        && url.host_str() == oidc.redirect.host_str()
        && url.port() == oidc.redirect.port()
        && url.path() == oidc.redirect.path();
    if !same_target {
        return Err(MatrixLoginFailure::InvalidCallback);
    }
    if Instant::now() > oidc.deadline {
        return Err(MatrixLoginFailure::Expired);
    }
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned());
    if state.as_deref() != Some(oidc.state.as_str()) {
        return Err(MatrixLoginFailure::InvalidCallback);
    }
    Ok(url)
}

fn is_finished(state: &MatrixLoginState) -> bool {
    matches!(
        state,
        MatrixLoginState::Succeeded { .. }
            | MatrixLoginState::Cancelled
            | MatrixLoginState::Failed { .. }
    )
}

fn device_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("matrix-login-device-name-required"));
    }
    Ok(name.to_owned())
}

/// 不透明的流程 ID，只用来认出是哪一次登录，不是凭据。
fn login_id() -> String {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("login-{hex}")
}

fn invalid(id: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: l10n!(CATALOG, language(), id),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
