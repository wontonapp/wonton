//! 一次 SAS 自我验证的操作与快照；SDK 完成后还要等私钥和备份确实可用。

use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use wonton_matrix::{Error, MatrixSession, SelfVerification};
use wonton_protocol::{
    Action, ChatAccount, ChatProblem, CoreError, DeviceVerification, VerificationCancel,
    VerificationFlow, VerificationStage,
};

use super::{ChatLink, invalid, lock, view};

// 秘密 gossip 需要多个同步往返；30 秒覆盖正常交换，又不让缺密钥的设备一直显示恢复中。
const KEYS_TIMEOUT: Duration = Duration::from_secs(30);
// 取钥匙期间再要几次缺的秘密：开始取后 1、3、7、15 秒（间隔翻倍），都在 KEYS_TIMEOUT 之内。验证完成时 SDK 要的那一次
// 可能比对方把本机记成“已验证”早到，被对方直接丢掉（ADR-0046）；正常 0.1–0.3 秒就取齐，用不到第一次。
const FIRST_ASK: Duration = Duration::from_secs(1);
// 期限过了还缺秘密时，从这一档起接着再要，每要一次翻倍、封顶 `LATE_ASK_MAX`：秘密多半还等在对方那里，
// 只是请求在对方认出本机之前被丢了。翻倍是为了别让停在“没取全”的页面一直敲服务端。
const LATE_ASK: Duration = Duration::from_secs(5);
const LATE_ASK_MAX: Duration = Duration::from_secs(60);

type Changes = Pin<Box<dyn Stream<Item = wonton_matrix::Result<VerificationFlow>> + Send>>;

pub(super) struct Control {
    pub(super) commands: mpsc::UnboundedSender<Command>,
    starting: bool,
    operating: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Accept,
    Confirm,
    Mismatch,
    Cancel,
}

pub(super) enum Command {
    Start(String),
    Operate(String, Operation),
}

struct Flow {
    verification: SelfVerification,
    changes: Changes,
    keys_deadline: Option<Instant>,
    /// 这次取钥匙已经再要过几次缺的秘密，见 [`ask_again`]。
    asked: u32,
    /// 下一次再要缺的秘密的时间。
    next_ask: Option<Instant>,
    /// 期限到了仍缺，已经以 `KeysMissing` 结束、还在等迟到的秘密。等到就改成完成。
    late: bool,
    /// 期限过后又要过几次，决定下一次的间隔（见 [`LATE_ASK`]）。
    late_asked: u32,
    finished: bool,
}

impl ChatLink {
    /// 重置恢复前取消进行中的确认（ADR-0076）：取消发不出去（没在跑、任务没了）就什么都不做，重置照常。
    /// 流程还在它自己的任务里收尾，对端随后收到“对方取消”。
    pub(super) fn cancel_active_verification(&self) {
        let flow = self.view.borrow().encryption.verification_flow.clone();
        if let Some(flow) = flow
            && active(&flow.stage)
            && let Some(control) = lock(&self.verification).as_mut()
        {
            let _ = control
                .commands
                .send(Command::Operate(flow.flow_id, Operation::Cancel));
        }
    }

    /// 设备确认的几个 `Action`。
    pub(super) fn verification_action(&self, action: Action) -> Result<(), CoreError> {
        let (flow_id, operation) = match action {
            Action::StartDeviceVerification => return self.start_verification(),
            Action::AcceptDeviceVerification { flow_id } => (flow_id, Operation::Accept),
            Action::ConfirmVerification { flow_id } => (flow_id, Operation::Confirm),
            Action::DenyVerification { flow_id } => (flow_id, Operation::Mismatch),
            Action::CancelDeviceVerification { flow_id } => (flow_id, Operation::Cancel),
            _ => unreachable!("dispatch only routes device-verification actions here"),
        };
        self.verification_operation(flow_id, operation)
    }

    /// 登录后跟着设备确认请求和“能不能用另一台设备确认”的任务。
    pub(super) fn verification_tasks(
        self: &Arc<Self>,
        session: &Arc<MatrixSession>,
    ) -> [tokio::task::JoinHandle<()>; 2] {
        [
            self.observe_verification(session.clone()),
            self.runtime
                .spawn(watch_availability(Arc::downgrade(self), session.clone())),
        ]
    }

    fn start_verification(&self) -> Result<(), CoreError> {
        self.current()?;
        let mut control = lock(&self.verification);
        let control = control.as_mut().ok_or_else(unavailable)?;
        let snapshot = self.view.borrow();
        let encryption = &snapshot.encryption;
        if !matches!(snapshot.account, ChatAccount::SignedIn { .. }) {
            return Err(unavailable());
        }
        if encryption.verification != DeviceVerification::Unverified
            || !encryption.can_verify_with_other_device
            || encryption
                .verification_flow
                .as_ref()
                .is_some_and(|flow| active(&flow.stage))
            || control.starting
        {
            return Err(unavailable());
        }
        let attempt = attempt_id()?;
        control.starting = true;
        if control.commands.send(Command::Start(attempt)).is_err() {
            control.starting = false;
            return Err(unavailable());
        }
        Ok(())
    }

    fn verification_operation(
        &self,
        flow_id: String,
        operation: Operation,
    ) -> Result<(), CoreError> {
        self.current()?;
        let mut control = lock(&self.verification);
        let control = control.as_mut().ok_or_else(unavailable)?;
        let snapshot = self.view.borrow();
        let encryption = &snapshot.encryption;
        if !matches!(snapshot.account, ChatAccount::SignedIn { .. }) {
            return Err(unavailable());
        }
        let flow = encryption
            .verification_flow
            .as_ref()
            .ok_or_else(unavailable)?;
        if flow.flow_id != flow_id {
            return Err(invalid("Verification flow ID does not match"));
        }
        let allowed = match operation {
            Operation::Accept => flow.stage == VerificationStage::Incoming,
            Operation::Confirm | Operation::Mismatch => {
                matches!(flow.stage, VerificationStage::Compare { .. })
            }
            Operation::Cancel => active(&flow.stage),
        };
        if !allowed || control.starting || control.operating {
            return Err(unavailable());
        }
        control.operating = true;
        if control
            .commands
            .send(Command::Operate(flow_id, operation))
            .is_err()
        {
            control.operating = false;
            return Err(unavailable());
        }
        Ok(())
    }

    fn observe_verification(
        self: &Arc<Self>,
        session: Arc<MatrixSession>,
    ) -> tokio::task::JoinHandle<()> {
        // 先注册处理器再启动同步，首批 to-device 请求不能丢。
        let incoming = session.incoming_verifications();
        let (commands, receiver) = mpsc::unbounded_channel();
        *lock(&self.verification) = Some(Control {
            commands,
            starting: false,
            operating: false,
        });
        self.runtime.spawn(run(
            Arc::downgrade(self),
            session,
            receiver,
            Box::pin(incoming),
        ))
    }
}

async fn run(
    chat: Weak<ChatLink>,
    session: Arc<MatrixSession>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut incoming: Pin<Box<dyn Stream<Item = wonton_matrix::Result<SelfVerification>> + Send>>,
) {
    let mut flow: Option<Flow> = None;
    let mut encryption = Box::pin(session.encryption_states());
    loop {
        let deadline = flow.as_ref().and_then(|flow| flow.keys_deadline);
        // 期限过了还在等迟到的秘密时也算“在等”，秘密一齐就当场收尾，不用等下一次状态变化。
        let waiting = deadline.is_some() || flow.as_ref().is_some_and(|flow| flow.late);
        let keys_ready = waiting && keys_ready(&session);
        let ask = flow
            .as_ref()
            .and_then(|flow| flow.next_ask)
            .filter(|_| !keys_ready);
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return };
                let Some(link) = current_link(&chat, &session) else { return };
                apply_command(&link, &session, &mut flow, command).await;
            }
            request = incoming.next() => {
                let Some(request) = request else { return };
                let Some(link) = current_link(&chat, &session) else { return };
                match request {
                    Ok(request) => receive(&link, &mut flow, request),
                    // 这是别的请求对象上的处理错误，与当前流程无关，不能把它取消掉。
                    Err(error) => report(&link, &error),
                }
            }
            change = next_change(&mut flow) => {
                let Some(link) = current_link(&chat, &session) else { return };
                match change {
                    Some(Ok(snapshot)) => update(&link, &mut flow, snapshot),
                    Some(Err(error)) => fail(&link, &mut flow, &error),
                    None => fail(&link, &mut flow, &Error::Upstream(
                        "Device verification stream ended before completion".into(),
                    )),
                }
            }
            state = encryption.next() => {
                if state.is_none() { return; }
                let Some(link) = current_link(&chat, &session) else { return };
                finish_keys(&link, &session, &mut flow, false);
            }
            () = key_deadline(deadline, keys_ready) => {
                let Some(link) = current_link(&chat, &session) else { return };
                finish_keys(&link, &session, &mut flow, !keys_ready);
            }
            () = sleep_until(ask) => ask_again(&session, &mut flow).await,
        }
    }
}

fn receive(link: &ChatLink, flow: &mut Option<Flow>, request: SelfVerification) {
    let _control = lock(&link.verification);
    let snapshot = match request.flow() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            report(link, &error);
            return;
        }
    };
    if let Some(current) = flow.as_ref() {
        let previous = match current.verification.flow() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                fail(link, flow, &error);
                return;
            }
        };
        let same_flow = previous.flow_id == snapshot.flow_id;
        if same_flow {
            if current.finished && snapshot.stage == VerificationStage::FetchingKeys {
                return;
            }
            update(link, flow, snapshot);
            return;
        }
        if !current.finished {
            update(link, flow, previous);
        }
        if !active(&snapshot.stage) {
            return;
        }
    }
    install(link, flow, request);
}

async fn apply_command(
    link: &ChatLink,
    session: &Arc<MatrixSession>,
    flow: &mut Option<Flow>,
    command: Command,
) {
    match command {
        Command::Start(attempt) => {
            *flow = None;
            let result = session.request_self_verification().await;
            if !current_session(link, session) {
                return;
            }
            // 发布结果和放开 `starting` 在同一把锁里（同 `receive`）：壳一看到失败就重试时，
            // `start_verification` 不会撞上还没放开的 `starting`、报“没准备好”。
            let mut control = lock(&link.verification);
            match result {
                Ok(request) => install(link, flow, request),
                Err(error) => {
                    set_snapshot(
                        link,
                        VerificationFlow {
                            flow_id: attempt,
                            started_here: true,
                            other_device_id: String::new(),
                            other_device_name: None,
                            stage: VerificationStage::Cancelled {
                                reason: VerificationCancel::Failed,
                            },
                        },
                    );
                    report(link, &error);
                }
            }
            if let Some(control) = control.as_mut() {
                control.starting = false;
            }
        }
        Command::Operate(flow_id, operation) => {
            let fetching = flow
                .as_ref()
                .is_some_and(|flow| flow.keys_deadline.is_some());
            if fetching && matches!(operation, Operation::Cancel) {
                let mut control = lock(&link.verification);
                link.view.send_modify(|view| {
                    if let Some(snapshot) = view.encryption.verification_flow.as_mut() {
                        snapshot.stage = VerificationStage::Cancelled {
                            reason: VerificationCancel::ByMe,
                        };
                    }
                    super::recovery::refresh(&mut view.encryption);
                });
                if let Some(flow) = flow.as_mut() {
                    flow.finished = true;
                    flow.keys_deadline = None;
                    flow.late = false;
                    flow.next_ask = None;
                }
                if let Some(control) = control.as_mut() {
                    control.operating = false;
                }
                return;
            }
            if let Some(verification) = flow.as_ref().map(|flow| flow.verification.clone()) {
                let result = match verification.flow() {
                    Ok(snapshot) if snapshot.flow_id == flow_id => match operation {
                        Operation::Accept => verification.accept().await,
                        Operation::Confirm => verification.confirm().await,
                        Operation::Mismatch => verification.mismatch().await,
                        Operation::Cancel => verification.cancel().await,
                    },
                    Ok(_) => Err(Error::InvalidInput(
                        "Verification flow ID does not match".into(),
                    )),
                    Err(error) => Err(error),
                };
                if !current_session(link, session) {
                    return;
                }
                let mut control = lock(&link.verification);
                match result {
                    Ok(()) => match verification.flow() {
                        Ok(snapshot) => update(link, flow, snapshot),
                        Err(error) => fail(link, flow, &error),
                    },
                    Err(error) => fail(link, flow, &error),
                }
                if let Some(control) = control.as_mut() {
                    control.operating = false;
                }
                return;
            }
            if let Some(control) = lock(&link.verification).as_mut() {
                control.operating = false;
            }
        }
    }
}

fn install(link: &ChatLink, flow: &mut Option<Flow>, verification: SelfVerification) {
    let changes = Box::pin(verification.changes());
    match verification.flow() {
        Ok(snapshot) => {
            *flow = Some(Flow {
                verification,
                changes,
                keys_deadline: None,
                asked: 0,
                next_ask: None,
                late: false,
                late_asked: 0,
                finished: false,
            });
            update(link, flow, snapshot);
        }
        Err(error) => report(link, &error),
    }
}

fn update(link: &ChatLink, flow: &mut Option<Flow>, snapshot: VerificationFlow) {
    let stage = &snapshot.stage;
    if let Some(flow) = flow.as_mut() {
        if flow.keys_deadline.is_some()
            && active(stage)
            && *stage != VerificationStage::FetchingKeys
        {
            return;
        }
        if *stage == VerificationStage::FetchingKeys && flow.keys_deadline.is_none() {
            // 又进到“正在取回密钥”：新的期限从这一刻算，没过期限的再要间隔也从头来。
            flow.keys_deadline = Some(Instant::now() + KEYS_TIMEOUT);
            flow.asked = 0;
            flow.next_ask = Some(Instant::now() + FIRST_ASK);
            flow.late = false;
            flow.late_asked = 0;
        }
    }
    let terminal = !active(stage);
    set_snapshot(link, snapshot);
    if terminal && let Some(flow) = flow.as_mut() {
        flow.finished = true;
        flow.keys_deadline = None;
        flow.late = false;
        flow.next_ask = None;
    }
}

/// 取钥匙的期限到了就以 `KeysMissing` 结束，之后秘密才到齐就改成完成。
///
/// 以 `KeysMissing` 结束后 `keys_deadline` 已经清掉，迟到的秘密靠 `late` 继续认：清掉以后的加密状态变化仍要
/// 走到这里，不然迟到的秘密到齐了也停在“没取全”。
fn finish_keys(link: &ChatLink, session: &MatrixSession, flow: &mut Option<Flow>, expired: bool) {
    let Some(waiting) = flow.as_ref() else { return };
    if waiting.keys_deadline.is_none() && !waiting.late {
        return;
    }
    let state = session.encryption_state();
    let complete = keys_ready(session);
    if !complete && !expired {
        return;
    }
    link.view.send_modify(|view| {
        // 读取 SDK 的同一份状态，Done 不会先于快照上的 Verified + Enabled。
        view.encryption.verification = view::verification(state.verification);
        view.encryption.recovery = view::recovery(state.recovery);
        if complete {
            view.encryption.can_verify_with_other_device = false;
        }
        super::recovery::refresh(&mut view.encryption);
        if let Some(snapshot) = view.encryption.verification_flow.as_mut() {
            snapshot.stage = if complete {
                VerificationStage::Done
            } else {
                VerificationStage::Cancelled {
                    reason: VerificationCancel::KeysMissing,
                }
            };
        }
    });
    if let Some(flow) = flow.as_mut() {
        flow.finished = true;
        flow.keys_deadline = None;
        flow.late = !complete;
        flow.next_ask = (!complete).then(|| Instant::now() + LATE_ASK);
    }
}

async fn next_change(flow: &mut Option<Flow>) -> Option<wonton_matrix::Result<VerificationFlow>> {
    match flow {
        Some(flow) if !flow.finished && flow.keys_deadline.is_none() => flow.changes.next().await,
        _ => std::future::pending().await,
    }
}

async fn key_deadline(deadline: Option<Instant>, ready: bool) {
    if ready {
        return;
    }
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// 排下一次再要缺的秘密的时间：期限之内按开始取钥匙后 `FIRST_ASK` × (2ⁿ⁺¹ − 1) 的间隔翻倍。
fn schedule_ask(flow: &mut Flow) {
    let Some(deadline) = flow.keys_deadline else {
        return;
    };
    let started = deadline - KEYS_TIMEOUT;
    let at = started + FIRST_ASK * ((2u32 << flow.asked.min(16)) - 1);
    flow.next_ask = (at < deadline).then_some(at);
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// 缺的秘密再要一次。要不成只记日志，照常到期以 `KeysMissing` 结束。
///
/// 以 `KeysMissing` 结束后接着按 [`LATE_ASK`] 翻倍再要：对方在把本机记成“已验证”之前收到的请求会直接丢掉
/// （matrix-sdk 0.19.1），丢了又没人再要，秘密就永远到不齐，页面上一直停在“没取全”。
async fn ask_again(session: &MatrixSession, flow: &mut Option<Flow>) {
    let Some(flow) = flow.as_mut() else { return };
    flow.next_ask = None;
    let late = flow.late;
    if late {
        let wait = LATE_ASK
            .saturating_mul(1 << flow.late_asked.min(4))
            .min(LATE_ASK_MAX);
        flow.late_asked += 1;
        flow.next_ask = Some(Instant::now() + wait);
    } else {
        flow.asked += 1;
        schedule_ask(flow);
    }
    let asked = if late { flow.late_asked } else { flow.asked };
    match session.request_missing_secrets().await {
        Ok(requested) => {
            tracing::info!(
                asked,
                late,
                requested,
                "asked other devices again for missing secrets"
            )
        }
        Err(error) => {
            tracing::warn!(%error, "could not ask other devices again for missing secrets")
        }
    }
}

fn keys_ready(session: &MatrixSession) -> bool {
    let state = session.encryption_state();
    state.verification == wonton_matrix::Verification::Verified
        && state.recovery == wonton_matrix::Recovery::Enabled
}

fn fail(link: &ChatLink, flow: &mut Option<Flow>, error: &Error) {
    link.view.send_modify(|view| {
        if let Some(snapshot) = view.encryption.verification_flow.as_mut() {
            snapshot.stage = VerificationStage::Cancelled {
                reason: VerificationCancel::Failed,
            };
        }
        super::recovery::refresh(&mut view.encryption);
    });
    if let Some(flow) = flow.as_mut() {
        flow.finished = true;
        flow.keys_deadline = None;
        flow.late = false;
        flow.next_ask = None;
    }
    report(link, error);
}

fn report(link: &ChatLink, error: &Error) {
    tracing::info!(%error, "Device verification failed");
    link.view.send_modify(|view| {
        view.problem = Some(ChatProblem {
            reason: view::failure(error),
            message: error.to_string(),
        })
    });
}

fn set_snapshot(link: &ChatLink, snapshot: VerificationFlow) {
    link.view.send_if_modified(|view| {
        if view.encryption.verification_flow.as_ref() == Some(&snapshot) {
            return false;
        }
        view.encryption.verification_flow = Some(snapshot);
        super::recovery::refresh(&mut view.encryption);
        true
    });
}

fn current_link(chat: &Weak<ChatLink>, session: &Arc<MatrixSession>) -> Option<Arc<ChatLink>> {
    let link = chat.upgrade()?;
    current_session(&link, session).then_some(link)
}

/// 会话还是当前登录的这一个吗？断线重连会换会话并中止旧任务；旧任务里在途的 SDK 调用
/// 返回后不能把旧会话的结果写进现在的视图。
fn current_session(link: &ChatLink, session: &Arc<MatrixSession>) -> bool {
    lock(&link.session)
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, session))
}

fn active(stage: &VerificationStage) -> bool {
    !matches!(
        stage,
        VerificationStage::Done | VerificationStage::Cancelled { .. }
    )
}

fn unavailable() -> CoreError {
    CoreError::NotReady {
        reason: "Device verification is not ready for this operation".into(),
    }
}

fn attempt_id() -> Result<String, CoreError> {
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).map_err(|error| CoreError::Upstream {
        reason: error.to_string(),
    })?;
    Ok(format!("attempt-{:032x}", u128::from_ne_bytes(id)))
}

async fn watch_availability(chat: Weak<ChatLink>, session: Arc<MatrixSession>) {
    let changes = match session.own_devices_changed().await {
        Ok(changes) => changes,
        Err(error) => {
            if let Some(link) = current_link(&chat, &session) {
                report(&link, &error);
            }
            return;
        }
    };
    let mut changes = std::pin::pin!(changes);
    let mut states = std::pin::pin!(session.encryption_states());
    loop {
        let available =
            if session.encryption_state().verification == wonton_matrix::Verification::Unverified {
                session.has_devices_to_verify_against().await
            } else {
                Ok(false)
            };
        let Some(link) = current_link(&chat, &session) else {
            return;
        };
        match available {
            Ok(available) => {
                link.view.send_modify(|view| {
                    view.encryption.can_verify_with_other_device = available
                        && session.encryption_state().verification
                            == wonton_matrix::Verification::Unverified;
                    super::recovery::refresh(&mut view.encryption);
                });
            }
            Err(error) => {
                link.view.send_modify(|view| {
                    view.encryption.can_verify_with_other_device = false;
                    super::recovery::refresh(&mut view.encryption);
                });
                report(&link, &error);
            }
        }
        drop(link);
        tokio::select! {
            change = changes.next() => if change.is_none() { return; },
            state = states.next() => if state.is_none() { return; },
        }
    }
}
