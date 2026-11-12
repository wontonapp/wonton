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
    finished: bool,
}

impl ChatLink {
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
        let keys_ready = deadline.is_some() && keys_ready(&session);
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
            if let Some(control) = lock(&link.verification).as_mut() {
                control.starting = false;
            }
        }
        Command::Operate(flow_id, operation) => {
            let fetching = flow
                .as_ref()
                .is_some_and(|flow| flow.keys_deadline.is_some());
            if fetching && matches!(operation, Operation::Cancel) {
                link.view.send_modify(|view| {
                    if let Some(snapshot) = view.encryption.verification_flow.as_mut() {
                        snapshot.stage = VerificationStage::Cancelled {
                            reason: VerificationCancel::ByMe,
                        };
                    }
                });
                if let Some(flow) = flow.as_mut() {
                    flow.finished = true;
                    flow.keys_deadline = None;
                }
                if let Some(control) = lock(&link.verification).as_mut() {
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
                match result {
                    Ok(()) => match verification.flow() {
                        Ok(snapshot) => update(link, flow, snapshot),
                        Err(error) => fail(link, flow, &error),
                    },
                    Err(error) => fail(link, flow, &error),
                }
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
        if *stage == VerificationStage::FetchingKeys {
            flow.keys_deadline
                .get_or_insert_with(|| Instant::now() + KEYS_TIMEOUT);
        }
    }
    let terminal = !active(stage);
    set_snapshot(link, snapshot);
    if terminal && let Some(flow) = flow.as_mut() {
        flow.finished = true;
        flow.keys_deadline = None;
    }
}

fn finish_keys(link: &ChatLink, session: &MatrixSession, flow: &mut Option<Flow>, expired: bool) {
    if !flow
        .as_ref()
        .is_some_and(|flow| flow.keys_deadline.is_some())
    {
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
        super::recovery::update_reminder(&mut view.encryption);
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
    });
    if let Some(flow) = flow.as_mut() {
        flow.finished = true;
        flow.keys_deadline = None;
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
                });
            }
            Err(error) => {
                link.view
                    .send_modify(|view| view.encryption.can_verify_with_other_device = false);
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
