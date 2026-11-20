//! 这台设备的人（ADR-0045）：设置页和填资料的页面读写自己的资料，家人的设备写的是自己的那条。

use wonton_l10n::l10n;
use wonton_matrix::{MatrixSession, RoomId, UserId};
use wonton_protocol::{
    BotRef, CoreError, FamilyAction, MY_PRIVATE_BOT_PATH, Me, OwnBotFailure, OwnBotProblem,
    OwnPrivateBotResponse, OwnPrivateBotResult, OwnPrivateBotView, OwnerProfileUpdate,
    PluginFailure,
};

use super::PluginsLink;
use super::api::{CoreApi, CoreApiError, READ_TIMEOUT, to_core_error};
use crate::l10n::{CATALOG, language};

const PATH: [&str; 1] = ["me"];

fn private_bot_path() -> [&'static str; 2] {
    let mut segments = MY_PRIVATE_BOT_PATH
        .strip_prefix("/_wonton/api/")
        .expect("private Bot path must use the core API prefix")
        .split('/');
    match (segments.next(), segments.next(), segments.next()) {
        (Some(first), Some(second), None) if !first.is_empty() && !second.is_empty() => {
            [first, second]
        }
        _ => panic!("private Bot path must have two nonempty segments"),
    }
}

impl PluginsLink {
    pub(crate) async fn me(&self) -> Result<Me, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&PATH, &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 校验在家里服务器的核心里做：名字和邮箱的规则只写一处，不合规则的原因原样带回来。
    pub(crate) async fn save_profile(&self, update: OwnerProfileUpdate) -> Result<Me, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.put(&PATH, &update, READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 自己的头像：壳传裁好、压好的文件，成功后的 `Me` 由下一次取资料带出来。协议先接住，上传和写 `avatar_url`
    /// 还没接上时明确报还不支持，不安静地吞掉。
    pub(crate) fn dispatch_my_avatar(&self, action: FamilyAction) -> Result<(), CoreError> {
        match action {
            FamilyAction::SetMyAvatar { .. } | FamilyAction::ClearMyAvatar => {
                Err(CoreError::NotReady {
                    reason: "This core cannot change your avatar yet".to_owned(),
                })
            }
            _ => Err(CoreError::InvalidInput {
                reason: "not a profile avatar action".to_owned(),
            }),
        }
    }
}

impl PluginsLink {
    pub(crate) async fn own_private_bot(
        &self,
        session: &MatrixSession,
    ) -> Result<OwnPrivateBotView, OwnBotProblem> {
        let api = self
            .api()
            .map_err(|error| own_bot_problem(CoreApiError::Transport(error)))?;
        fetch_own_private_bot(&api, session.user_id()).await
    }

    /// 测试接缝：真会话要 login/whoami/SDK restore 才建得出，测试用指定核心地址按指定本人
    /// 走与 [`PluginsLink::own_private_bot`] 相同的取数和校验；生产路径不经过这里。
    #[cfg(test)]
    pub(crate) async fn own_private_bot_at(
        base: &str,
        session_user_id: &UserId,
    ) -> Result<OwnPrivateBotView, OwnBotProblem> {
        let api = CoreApi {
            base: base.parse().expect("test core base URL"),
            client: reqwest::Client::new(),
            languages: String::new(),
            version: super::api::ServerVersion::default(),
        };
        fetch_own_private_bot(&api, session_user_id).await
    }
}

async fn fetch_own_private_bot(
    api: &CoreApi,
    session_user_id: &UserId,
) -> Result<OwnPrivateBotView, OwnBotProblem> {
    let response: OwnPrivateBotResponse = api
        .get_detailed(&private_bot_path(), &[], READ_TIMEOUT)
        .await
        .map_err(own_bot_problem)?;
    validate_own_private_bot(&response, session_user_id)
        .map_err(|kind| private_bot_protocol_problem(Some(kind)))?;
    Ok(match response.result {
        OwnPrivateBotResult::NotApplicable => OwnPrivateBotView::NotApplicable,
        OwnPrivateBotResult::Absent => OwnPrivateBotView::Absent,
        OwnPrivateBotResult::Ready(identity) => OwnPrivateBotView::Ready {
            bot: identity.bot,
            user_id: identity.user_id,
            home_room: identity.home_room,
            room_problem: identity.room_problem,
        },
        OwnPrivateBotResult::Other => OwnPrivateBotView::Other,
    })
}

fn validate_own_private_bot(
    response: &OwnPrivateBotResponse,
    session_user_id: &UserId,
) -> Result<(), &'static str> {
    let requester =
        <&UserId>::try_from(response.requester_user_id.as_str()).map_err(|_| "identity_invalid")?;
    if requester != session_user_id {
        return Err("requester_mismatch");
    }
    if let OwnPrivateBotResult::Ready(identity) = &response.result {
        let user_id =
            <&UserId>::try_from(identity.user_id.as_str()).map_err(|_| "identity_invalid")?;
        if user_id == requester
            || user_id.server_name() != requester.server_name()
            || !valid_bot_ref(&identity.bot)
            || identity
                .home_room
                .as_deref()
                .is_some_and(|room| <&RoomId>::try_from(room).is_err())
        {
            return Err("identity_invalid");
        }
    }
    Ok(())
}

fn valid_bot_ref(bot: &BotRef) -> bool {
    let mut labels = bot.plugin_id.split('.');
    bot.plugin_id.len() <= 63
        && labels.next().is_some_and(valid_label)
        && labels.next().is_some_and(valid_label)
        && labels.all(valid_label)
        && bot.bot.len() <= 32
        && valid_label(&bot.bot)
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn private_bot_protocol_problem(kind: Option<&str>) -> OwnBotProblem {
    OwnBotProblem {
        category: OwnBotFailure::Protocol,
        kind: kind.map(str::to_owned),
        status: None,
        detail: l10n!(
            CATALOG,
            language(),
            "plugins-api-invalid-response",
            reason = "private_bot_response"
        ),
    }
}

fn own_bot_problem(error: CoreApiError) -> OwnBotProblem {
    match error {
        CoreApiError::Transport(problem) => {
            let category = match problem.reason {
                PluginFailure::TransportUnavailable => OwnBotFailure::TransportUnavailable,
                PluginFailure::Unauthorized => OwnBotFailure::Unauthorized,
                PluginFailure::Protocol => OwnBotFailure::Protocol,
                PluginFailure::Upstream => OwnBotFailure::Upstream,
                _ => OwnBotFailure::Other,
            };
            let detail = if category == OwnBotFailure::TransportUnavailable {
                l10n!(CATALOG, language(), "plugins-api-not-connected")
            } else {
                l10n!(
                    CATALOG,
                    language(),
                    "plugins-api-http-client",
                    reason = "private_bot_connection"
                )
            };
            OwnBotProblem {
                category,
                kind: None,
                status: None,
                detail,
            }
        }
        CoreApiError::Http { status, kind, .. } => OwnBotProblem {
            category: own_bot_http_failure(status, kind.as_deref()),
            kind,
            status: Some(status),
            detail: l10n!(
                CATALOG,
                language(),
                "plugins-api-server-status",
                status = status
            ),
        },
        CoreApiError::Protocol(_) => private_bot_protocol_problem(None),
    }
}

fn own_bot_http_failure(status: u16, kind: Option<&str>) -> OwnBotFailure {
    match kind {
        Some("unauthenticated") => OwnBotFailure::Unauthorized,
        Some("cross_site" | "permission_required" | "permission_denied") => {
            OwnBotFailure::PermissionDenied
        }
        Some("bot_record_unavailable") => OwnBotFailure::StorageUnavailable,
        Some("bot_record_corrupt") => OwnBotFailure::StorageCorrupt,
        Some(
            "private_bot_conflict"
            | "private_bot_ref_invalid"
            | "private_bot_identity_mismatch"
            | "member_identity_invalid",
        ) => OwnBotFailure::RecordInconsistent,
        // `role_not_found` 只预留给别的 Bot 投影：本人路由缺角色返回 `member_role_unavailable`，
        // 谁真发 `role_not_found` 都按未就绪算。
        Some(
            "private_bot_pending"
            | "private_bot_changed"
            | "private_bot_record_missing"
            | "private_bot_keeper_unknown"
            | "member_identity_unavailable"
            | "member_role_unavailable"
            | "matrix_unavailable"
            | "role_not_found",
        ) => OwnBotFailure::NotReady,
        None if status == 403 || status == 502 => OwnBotFailure::TransportUnavailable,
        _ if status == 401 => OwnBotFailure::Unauthorized,
        _ if status == 404 => OwnBotFailure::CapabilityUnknown,
        _ if status >= 500 => OwnBotFailure::Upstream,
        _ => OwnBotFailure::Other,
    }
}

#[cfg(test)]
#[path = "me_private_bot_tests.rs"]
mod me_private_bot_tests;
