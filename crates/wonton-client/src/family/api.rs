//! `/_wonton/api/members`、`/_wonton/api/roles` 和加入请求的请求：走本地代理，核心凭隧道身份认出这台设备。
//!
//! 失败按核心的类别分开：权限不够、对方撤了或码过期（`not_found`）、人已经移除……说明由核心按 App 语言写好。
//! 本地代理自己回的错误没有类别，按状态码分。

use std::time::Duration;

use reqwest::Method;
use serde::de::DeserializeOwned;
use wonton_protocol::{FamilyFailure, FamilyProblem, PermissionRequired};

use crate::l10n::text;
use crate::plugins::api::CoreApi;

/// 移除要吊销设备、停用账号，可能还要删私人空间和 Bot，比取列表慢。
pub(super) const REMOVE_TIMEOUT: Duration = Duration::from_secs(120);

impl CoreApi {
    pub(super) async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<&(impl serde::Serialize + ?Sized)>,
        timeout: Duration,
    ) -> Result<T, FamilyProblem> {
        let bytes = self.call_empty(method, segments, body, timeout).await?;
        serde_json::from_slice(&bytes).map_err(|error| FamilyProblem {
            reason: FamilyFailure::Protocol,
            message: text(
                "family-invalid-response",
                &[("reason", error.to_string().into())],
            ),
            missing: None,
        })
    }

    /// 不要响应体的接口：作废、同意、拒绝回 204。
    pub(super) async fn call_empty(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<&(impl serde::Serialize + ?Sized)>,
        timeout: Duration,
    ) -> Result<Vec<u8>, FamilyProblem> {
        let bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| FamilyProblem {
                reason: FamilyFailure::Protocol,
                message: text(
                    "family-request-body",
                    &[("reason", error.to_string().into())],
                ),
                missing: None,
            })?;
        let url = self.url(segments, &[]);
        let (status, kind, bytes) = self
            .exchange_detail(method, url, bytes, "application/json", timeout)
            .await
            .map_err(|problem| FamilyProblem {
                reason: FamilyFailure::TransportUnavailable,
                message: text("family-transport", &[("reason", problem.message.into())]),
                missing: None,
            })?;
        if status < 400 {
            return Ok(bytes);
        }
        Err(problem(status, kind.as_deref(), &bytes))
    }
}

/// 按状态码、`X-Wonton-Error` 和错误体分类。`permission_required` 的响应体是
/// [`PermissionRequired`]，不是错误信封。
fn problem(status: u16, kind: Option<&str>, body: &[u8]) -> FamilyProblem {
    let body: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    // 核心把类别写在 `X-Wonton-Error` 头和错误体里；本地代理回的没有头，只有状态码。
    let kind = kind
        .map(str::to_owned)
        .or_else(|| body["error"]["kind"].as_str().map(str::to_owned));
    let message = body["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            text(
                "family-server-status",
                &[("status", status.to_string().into())],
            )
        });
    let missing = || {
        serde_json::from_value::<PermissionRequired>(body.clone())
            .ok()
            .map(|required| required.missing)
    };
    let (reason, missing) = match kind.as_deref() {
        Some("permission_required") => (FamilyFailure::PermissionRequired, missing()),
        Some("unauthenticated" | "cross_site") => (FamilyFailure::Unauthorized, None),
        Some("not_found" | "role_not_found") => (FamilyFailure::NotFound, None),
        // `/me` 报的是 member_removed，成员和角色接口报 removed，同一个意思。
        Some("member_removed" | "removed") => (FamilyFailure::Removed, None),
        Some("device_registration") => (FamilyFailure::DeviceRegistered, None),
        Some("username_taken") => (FamilyFailure::UsernameTaken, None),
        Some("matrix_unavailable") => (FamilyFailure::MatrixUnavailable, None),
        Some("not_ready") => (FamilyFailure::NotReady, None),
        Some("invalid_input") => (FamilyFailure::InvalidInput, None),
        Some("storage") => (FamilyFailure::Storage, None),
        Some(_) if status >= 500 => (FamilyFailure::Upstream, None),
        Some(_) => (FamilyFailure::Other, None),
        None if status == 403 => (FamilyFailure::PermissionRequired, missing()),
        None if status == 401 => (FamilyFailure::Unauthorized, None),
        None if status == 404 => (FamilyFailure::NotFound, None),
        None if status >= 500 => (FamilyFailure::Upstream, None),
        None => (FamilyFailure::Other, None),
    };
    FamilyProblem {
        reason,
        message,
        missing,
    }
}

/// 没身份时（没有代理或隧道不可用）的失败，不经过网络。
pub(super) fn not_connected() -> FamilyProblem {
    FamilyProblem {
        reason: FamilyFailure::TransportUnavailable,
        message: text("family-not-connected", &[]),
        missing: None,
    }
}
