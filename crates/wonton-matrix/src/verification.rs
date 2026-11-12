//! SAS 自我验证；信任和秘密交换由 SDK 完成，不提供直接签名设备的捷径。

use std::sync::{Arc, Mutex};

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    Client,
    encryption::verification::{
        SasState, SasVerification, VerificationRequest, VerificationRequestState,
    },
    ruma::events::key::verification::VerificationMethod,
};
use tokio::sync::Mutex as AsyncMutex;
use wonton_protocol::{VerificationCancel, VerificationFlow, VerificationStage};

use crate::{Error, MatrixSession, Result};

pub(crate) mod concurrency;
mod mapping;

type RequestChanges = futures_util::stream::BoxStream<'static, VerificationRequestState>;
type SasChanges = futures_util::stream::BoxStream<'static, SasState>;

/// 一次标准 SAS 自我验证。轮询 [`Self::changes`] 驱动协议协商，但不会代替主人接受请求或确认表情。
#[derive(Clone)]
pub struct SelfVerification {
    inner: Arc<Inner>,
}

struct Inner {
    request: VerificationRequest,
    client: Client,
    snapshot: Mutex<Snapshot>,
    // SDK 操作可能先改变本地状态、再等待 HTTP；同一 flow 的自动协商和显式操作必须串行。
    control: Arc<AsyncMutex<Control>>,
}

struct Snapshot {
    other_device_id: String,
    other_device_name: Option<String>,
    sas: Option<SasVerification>,
    failure: Option<Error>,
    explicit_cancel: bool,
    concurrent_remote: bool,
    concurrent_notified: bool,
}

#[derive(Default)]
struct Control {
    accepted: bool,
    sas_started: bool,
    sas_accepted: bool,
}

struct Changes {
    verification: SelfVerification,
    requests: RequestChanges,
    sas: Option<SasChanges>,
    last: Option<VerificationFlow>,
    ended: bool,
}

impl MatrixSession {
    /// 向自己账号的已交叉签名设备请求 SAS 验证，不预选广播的接收设备。
    pub async fn request_self_verification(&self) -> Result<SelfVerification> {
        self.client
            .encryption()
            .wait_for_e2ee_initialization_tasks()
            .await;
        let identity = self
            .client
            .encryption()
            .request_user_identity(self.user_id())
            .await?
            .ok_or_else(|| Error::NotReady("账号的交叉签名身份尚未就绪".to_owned()))?;
        let request = identity
            .request_verification_with_methods(vec![VerificationMethod::SasV1])
            .await
            .map_err(|error| match error {
                matrix_sdk::encryption::identities::RequestVerificationError::Sdk(error) => {
                    error.into()
                }
                other => Error::Upstream(other.to_string()),
            })?;
        let verification = self.verifications.wrap(request, &self.client)?;
        self.verifications.notify_collisions().await;
        Ok(verification)
    }

    /// 调用时立即注册，须在启动同步前调用；释放流即解除处理器，不留 Client 引用环。
    /// 同 flow_id 可再次送达以细化已取消原因；保留当前终态的 wrapper，直到下一流程替换它。
    pub fn incoming_verifications(
        &self,
    ) -> impl Stream<Item = Result<SelfVerification>> + Send + use<> {
        concurrency::incoming(&self.client, &self.verifications)
    }

    /// 是否有另一台已交叉签名、具备加密身份且非休眠的候选设备；不代表它在线或持有全部秘密。
    pub async fn has_devices_to_verify_against(&self) -> Result<bool> {
        self.client
            .encryption()
            .wait_for_e2ee_initialization_tasks()
            .await;
        Ok(self
            .client
            .encryption()
            .has_devices_to_verify_against()
            .await?)
    }

    /// 自己账号设备表发生变化时唤醒客户端，重新查询可验证设备，不轮询服务器。
    pub async fn own_devices_changed(&self) -> Result<impl Stream<Item = ()> + Send + use<>> {
        let encryption = self.client.encryption();
        encryption.wait_for_e2ee_initialization_tasks().await;
        let own_user = self.user_id().to_owned();
        let identity_user = own_user.clone();
        let devices = encryption
            .devices_stream()
            .await?
            .filter_map(move |updates| {
                future::ready(
                    (updates.new.contains_key(&own_user)
                        || updates.changed.contains_key(&own_user))
                    .then_some(()),
                )
            });
        let identities = encryption
            .user_identities_stream()
            .await?
            .filter_map(move |updates| {
                future::ready(
                    (updates.new.contains_key(&identity_user)
                        || updates.changed.contains_key(&identity_user))
                    .then_some(()),
                )
            });
        Ok(stream::select(devices, identities))
    }
}

impl SelfVerification {
    fn new(request: VerificationRequest, client: Client) -> Result<Self> {
        if !request.is_self_verification() || request.own_user_id() != request.other_user_id() {
            return Err(Error::InvalidInput("只允许自己账号的设备确认".to_owned()));
        }
        let verification = Self {
            inner: Arc::new(Inner {
                request,
                client,
                snapshot: Mutex::new(Snapshot {
                    other_device_id: String::new(),
                    other_device_name: None,
                    sas: None,
                    failure: None,
                    explicit_cancel: false,
                    concurrent_remote: false,
                    concurrent_notified: false,
                }),
                control: Arc::new(AsyncMutex::new(Control::default())),
            }),
        };
        verification.flow()?;
        Ok(verification)
    }

    /// 当前 SDK 状态；SAS 完成只表示正在取回密钥，最终结果由客户端结合恢复状态判定。
    pub fn flow(&self) -> Result<VerificationFlow> {
        let request = &self.inner.request;
        let state = request.state();
        let mut snapshot = self
            .inner
            .snapshot
            .lock()
            .expect("verification snapshot poisoned");
        if let Some(error) = &snapshot.failure {
            return Err(error.clone());
        }
        mapping::remember_peer(&state, &mut snapshot)?;
        let stage = if request.is_passive() {
            VerificationStage::Cancelled {
                reason: VerificationCancel::AcceptedElsewhere,
            }
        } else {
            mapping::stage(&state, &snapshot, request.we_started())?
        };
        Ok(VerificationFlow {
            flow_id: request.flow_id().to_owned(),
            started_here: request.we_started(),
            other_device_id: snapshot.other_device_id.clone(),
            other_device_name: snapshot.other_device_name.clone(),
            stage,
        })
    }

    /// 先给当前快照，再合并请求和 SAS 的变化；出错给一次原始类别的错误后结束。
    pub fn changes(&self) -> impl Stream<Item = Result<VerificationFlow>> + Send + use<> {
        // 先订阅，再读快照；切换到 SAS 时也遵守同样顺序，避免丢掉快速的状态转换。
        let requests = self.inner.request.changes().boxed();
        let changes = Changes {
            verification: self.clone(),
            requests,
            sas: None,
            last: None,
            ended: false,
        };
        stream::unfold(changes, |mut changes| async move {
            let value = changes.next().await?;
            Some((value, changes))
        })
    }

    /// 主人接受收到的请求；只协商 SAS。
    pub async fn accept(&self) -> Result<()> {
        let mut control = self.inner.control.lock().await;
        self.ensure_active()?;
        if self.inner.request.we_started()
            || !matches!(
                self.inner.request.state(),
                VerificationRequestState::Requested { .. }
            )
        {
            return Err(Error::NotReady("设备确认请求不在等待接受的状态".to_owned()));
        }
        let result = self
            .inner
            .request
            .accept_with_methods(vec![VerificationMethod::SasV1])
            .await;
        self.sdk_result(result)?;
        control.accepted = true;
        Ok(())
    }

    pub async fn confirm(&self) -> Result<()> {
        self.compare_operation(true).await
    }

    pub async fn mismatch(&self) -> Result<()> {
        self.compare_operation(false).await
    }

    /// 主人的显式取消须先标记，不能与 SDK 的自动并发取消混为一谈。
    pub async fn cancel(&self) -> Result<()> {
        let _control = self.inner.control.lock().await;
        // 对方的取消、确认可能比主人这一次点击先到：已经结束的流程再点取消按完成算。
        if !self.active()? {
            return Ok(());
        }
        {
            let mut snapshot = self
                .inner
                .snapshot
                .lock()
                .expect("verification snapshot poisoned");
            // Crypto 同步不持有 control；不要把已经发生的自动取消改记成手动取消。
            if self.inner.request.is_cancelled() {
                return Ok(());
            }
            snapshot.explicit_cancel = true;
        }
        self.sdk_result(self.inner.request.cancel().await)
    }

    async fn compare_operation(&self, matches: bool) -> Result<()> {
        let _control = self.inner.control.lock().await;
        // 两台设备可能几乎同时点：后到的一次落在已结束的流程上，快照里已经是对方的结论（例如表情对不上）。
        if !self.active()? {
            return Ok(());
        }
        let sas = self
            .sas()
            .ok_or_else(|| Error::NotReady("SAS 尚未开始".to_owned()))?;
        if !matches!(sas.state(), SasState::KeysExchanged { .. }) {
            return Err(Error::NotReady("尚未展示 SAS，或本设备已确认".to_owned()));
        }
        let result = if matches {
            sas.confirm().await
        } else {
            sas.mismatch().await
        };
        self.sdk_result(result)
    }

    fn ensure_active(&self) -> Result<()> {
        if !self.active()? {
            return Err(Error::NotReady("这次设备确认已经结束".to_owned()));
        }
        Ok(())
    }

    fn active(&self) -> Result<bool> {
        Ok(!mapping::terminal(&self.flow()?.stage))
    }

    fn sas(&self) -> Option<SasVerification> {
        self.inner
            .snapshot
            .lock()
            .expect("verification snapshot poisoned")
            .sas
            .clone()
    }

    fn sdk_result<T>(&self, result: matrix_sdk::Result<T>) -> Result<T> {
        result.map_err(|error| self.fail(error.into()))
    }

    fn fail(&self, error: Error) -> Error {
        self.inner
            .snapshot
            .lock()
            .expect("verification snapshot poisoned")
            .failure = Some(error.clone());
        error
    }

    async fn negotiate(&self) -> Result<()> {
        let mut control = self.inner.control.clone().lock_owned().await;
        if mapping::terminal(&self.flow()?.stage) {
            return Ok(());
        }
        enum Operation {
            Start,
            Accept(SasVerification),
        }
        let operation = if self.inner.request.we_started()
            && self.inner.request.is_ready()
            && !control.sas_started
        {
            control.sas_started = true;
            Operation::Start
        } else if let Some(sas) = self.sas()
            && !sas.we_started()
            && matches!(sas.state(), SasState::Started { .. })
            && control.accepted
            && !control.sas_accepted
        {
            control.sas_accepted = true;
            Operation::Accept(sas)
        } else {
            return Ok(());
        };
        // select! 的另一分支可能处理显式操作并等这把锁；发送不能依赖状态流继续被轮询。
        // SDK 在 HTTP 完成前改变本地状态，因此也不能取消这次发送、再按新状态重试。
        // JoinHandle 被丢弃时任务只是脱离（tokio 的 detach），发送自己完成并释放锁。
        let verification = self.clone();
        let mut task =
            tokio::spawn(async move {
                let _control = control;
                match operation {
                    Operation::Start => {
                        let started = verification.inner.request.start_sas().await;
                        match started {
                            Ok(Some(sas)) => {
                                verification
                                    .inner
                                    .snapshot
                                    .lock()
                                    .expect("verification snapshot poisoned")
                                    .sas = Some(sas);
                                Ok(())
                            }
                            // 请求在发起与发送之间被并发取消等终态抢先：如实让 SDK 状态说话，
                            // 不伪造一个发送失败盖住真实原因。
                            Ok(None)
                                if verification
                                    .flow()
                                    .is_ok_and(|flow| mapping::terminal(&flow.stage)) =>
                            {
                                Ok(())
                            }
                            Ok(None) => Err(verification
                                .fail(Error::Upstream("SDK 无法开始 SAS 验证".to_owned()))),
                            Err(error) => Err(verification.fail(error.into())),
                        }
                    }
                    Operation::Accept(sas) => verification.sdk_result(sas.accept().await),
                }
            });
        (&mut task)
            .await
            .map_err(|error| self.fail(Error::Upstream(error.to_string())))?
    }
}

impl Changes {
    async fn next(&mut self) -> Option<Result<VerificationFlow>> {
        while !self.ended {
            let result = match self.verification.flow() {
                Ok(_) if self.last.is_some() => self.verification.negotiate().await,
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                self.ended = true;
                return Some(Err(error));
            }
            if self.sas.is_none() {
                self.sas = self.verification.sas().map(|sas| sas.changes().boxed());
            }
            let current = self.verification.flow();
            match current {
                Ok(flow) => {
                    self.ended = mapping::terminal(&flow.stage);
                    if self.last.as_ref() != Some(&flow) {
                        self.last = Some(flow.clone());
                        return Some(Ok(flow));
                    }
                    if self.ended {
                        return None;
                    }
                }
                Err(error) => {
                    self.ended = true;
                    return Some(Err(error));
                }
            }
            let changed = if let Some(sas) = self.sas.as_mut() {
                tokio::select! {
                    state = self.requests.next() => state.is_some(),
                    state = sas.next() => state.is_some(),
                }
            } else {
                self.requests.next().await.is_some()
            };
            if !changed {
                self.ended = true;
                return Some(Err(self
                    .verification
                    .fail(Error::Upstream("SDK 的设备确认状态流意外结束".to_owned()))));
            }
        }
        None
    }
}
