//! SDK 的 m.user 不携带自动并发取消的原因；扩展取消码仅细化已取消的流程，不改变信任状态。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use futures_util::{Stream, stream};
use matrix_sdk::{
    Client,
    encryption::verification::{VerificationRequest, VerificationRequestState},
    ruma::{
        TransactionId,
        api::client::to_device::send_event_to_device,
        events::{
            ToDeviceEventType,
            key::verification::{
                cancel::{
                    CancelCode, ToDeviceKeyVerificationCancelEvent,
                    ToDeviceKeyVerificationCancelEventContent,
                },
                request::ToDeviceKeyVerificationRequestEvent,
            },
        },
        serde::Raw,
        to_device::DeviceIdOrAllDevices,
    },
};

use super::{Inner, SelfVerification};
use crate::{Error, Result};

pub(super) const CANCEL_CODE: &str = "org.wonton.concurrent_request";

#[derive(Default)]
pub(crate) struct Registry {
    flows: Mutex<HashMap<String, Weak<Inner>>>,
}

impl Registry {
    pub(super) fn wrap(
        &self,
        request: VerificationRequest,
        client: &Client,
    ) -> Result<SelfVerification> {
        let mut flows = self.flows.lock().expect("verification registry poisoned");
        flows.retain(|_, flow| flow.strong_count() > 0);
        if let Some(inner) = flows.get(request.flow_id()).and_then(Weak::upgrade) {
            return Ok(SelfVerification { inner });
        }
        let verification = SelfVerification::new(request, client.clone())?;
        flows.insert(
            verification.inner.request.flow_id().to_owned(),
            Arc::downgrade(&verification.inner),
        );
        Ok(verification)
    }

    fn get(&self, flow_id: &str) -> Option<SelfVerification> {
        let mut flows = self.flows.lock().expect("verification registry poisoned");
        flows.retain(|_, flow| flow.strong_count() > 0);
        flows
            .get(flow_id)
            .and_then(Weak::upgrade)
            .map(|inner| SelfVerification { inner })
    }

    pub(super) async fn notify_collisions(&self) {
        let flows: Vec<_> = {
            let mut flows = self.flows.lock().expect("verification registry poisoned");
            flows.retain(|_, flow| flow.strong_count() > 0);
            flows.values().filter_map(Weak::upgrade).collect()
        };
        for inner in flows {
            // 细化通知是尽力而为：一条发送失败不挡别的流程，也不影响新请求的创建。
            if let Err(error) = notify_collision(&SelfVerification { inner }).await {
                tracing::warn!(%error, "并发确认的原因细化没有送达");
            }
        }
    }
}

pub(super) fn incoming(
    client: &Client,
    registry: &Arc<Registry>,
) -> impl Stream<Item = Result<SelfVerification>> + Send + use<> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let requests = {
        let sender = sender.clone();
        let registry = registry.clone();
        client.add_event_handler(
            move |event: ToDeviceKeyVerificationRequestEvent, client: Client| {
                let sender = sender.clone();
                let registry = registry.clone();
                async move {
                    if client.user_id() != Some(event.sender.as_ref()) {
                        return;
                    }
                    let result = receive_request(event, &client, &registry).await;
                    let result = match result {
                        Ok(Some(verification)) => Ok(verification),
                        Ok(None) => return,
                        Err(error) => Err(error),
                    };
                    let _ = sender.send(result);
                }
            },
        )
    };
    let cancellations = {
        let registry = registry.clone();
        client.add_event_handler(
            move |event: ToDeviceKeyVerificationCancelEvent, client: Client| {
                let sender = sender.clone();
                let registry = registry.clone();
                async move {
                    if client.user_id() != Some(event.sender.as_ref())
                        || event.content.code.as_str() != CANCEL_CODE
                    {
                        return;
                    }
                    match receive_cancellation(event, &client, &registry).await {
                        Ok(Some(verification)) => {
                            let _ = sender.send(Ok(verification));
                        }
                        Ok(None) => (),
                        Err(error) => {
                            let _ = sender.send(Err(error));
                        }
                    }
                }
            },
        )
    };
    let guards = (
        client.event_handler_drop_guard(requests),
        client.event_handler_drop_guard(cancellations),
    );
    // 单个事件处理失败（比如过期的请求被 SDK 丢弃）只报一次错误，订阅和后续事件不受影响；
    // 流被调用方释放时 guard 才解除处理器。
    stream::unfold((receiver, guards), |(mut receiver, guards)| async move {
        let value = receiver.recv().await?;
        Some((value, (receiver, guards)))
    })
}

async fn receive_request(
    event: ToDeviceKeyVerificationRequestEvent,
    client: &Client,
    registry: &Registry,
) -> Result<Option<SelfVerification>> {
    let request = client
        .encryption()
        .get_verification_request(&event.sender, &event.content.transaction_id)
        .await;
    let verification = match request {
        Some(request) if request.is_self_verification() && !request.we_started() => {
            registry.wrap(request, client)?
        }
        Some(_) => return Ok(None),
        None => return Err(Error::Upstream("SDK 未保留收到的设备确认请求".to_owned())),
    };
    {
        let mut snapshot = verification
            .inner
            .snapshot
            .lock()
            .expect("verification snapshot poisoned");
        if snapshot.other_device_id.is_empty() {
            snapshot.other_device_id = event.content.from_device.to_string();
        }
    }
    registry.notify_collisions().await;
    Ok(Some(verification))
}

async fn receive_cancellation(
    event: ToDeviceKeyVerificationCancelEvent,
    client: &Client,
    registry: &Registry,
) -> Result<Option<SelfVerification>> {
    let verification =
        if let Some(verification) = registry.get(event.content.transaction_id.as_str()) {
            verification
        } else {
            // 请求操作等待 HTTP 期间，SDK 可能已经接收了取消，但 wrapper 尚未创建。
            let Some(request) = client
                .encryption()
                .get_verification_request(&event.sender, &event.content.transaction_id)
                .await
            else {
                return Ok(None);
            };
            registry.wrap(request, client)?
        };
    // SDK 忽略终态后的取消；只细化同 flow 的 m.user，不覆盖超时、不同、被接手或完成。
    if let VerificationRequestState::Cancelled(info) = verification.inner.request.state()
        && !info.cancelled_by_us()
        && (*info.cancel_code() == CancelCode::User || info.cancel_code().as_str() == CANCEL_CODE)
    {
        verification
            .inner
            .snapshot
            .lock()
            .expect("verification snapshot poisoned")
            .concurrent_remote = true;
        return Ok(Some(verification));
    }
    Ok(None)
}

pub(super) async fn notify_collision(verification: &SelfVerification) -> Result<()> {
    let request = &verification.inner.request;
    let Some(info) = request.cancel_info() else {
        return Ok(());
    };
    if *info.cancel_code() != CancelCode::User || !info.cancelled_by_us() {
        return Ok(());
    }
    let peer = {
        let mut snapshot = verification
            .inner
            .snapshot
            .lock()
            .expect("verification snapshot poisoned");
        if snapshot.explicit_cancel || snapshot.concurrent_notified {
            return Ok(());
        }
        snapshot.concurrent_notified = true;
        snapshot.other_device_id.clone()
    };
    let content = ToDeviceKeyVerificationCancelEventContent::new(
        request.flow_id().into(),
        "Concurrent verification requests cancelled by the SDK".to_owned(),
        CancelCode::from(CANCEL_CODE),
    );
    let content = Raw::new(&content)
        .map_err(|error| Error::Upstream(error.to_string()))?
        .cast();
    let target = if peer.is_empty() {
        DeviceIdOrAllDevices::AllDevices
    } else {
        DeviceIdOrAllDevices::DeviceId(peer.into())
    };
    let messages = [(request.own_user_id().to_owned(), [(target, content)].into())].into();
    let message = send_event_to_device::v3::Request::new_raw(
        ToDeviceEventType::KeyVerificationCancel,
        TransactionId::new(),
        messages,
    );
    // 流程已经终态，这里只是补一个原因分类；发送失败记日志，不把终态污染成错误。
    verification
        .inner
        .client
        .send(message)
        .await
        .map_err(Error::from)
        .map(drop)
}
