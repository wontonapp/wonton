//! 给壳挑的两张名单（ADR-0113 第 9 节）：能用哪些媒体源、能共享给谁。都只读，不改任何记录。

use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use hyper::HeaderMap;
use wonton_protocol::MemberStatus;
use wonton_protocol::media::choices::{
    MediaSource, MediaSources, ShareTarget, ShareTargets, SourcesQuery,
};

use super::{Caller, MediaQuery};
use crate::gateway::api::languages;
use crate::gateway::{Gateway, Person, matrix_user_id};

/// `GET /sources`：没点名来源时会从中挑的那几家，规则在 [`crate::media::provider::MediaProviders::sources`]。
pub(super) async fn sources(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    MediaQuery(query): MediaQuery<SourcesQuery>,
) -> Response {
    let langs = languages(&headers);
    let sources = caller
        .media
        .providers
        .sources(
            caller.allowed(&gateway),
            &query.operation,
            query.kind.as_ref(),
        )
        .into_iter()
        .map(|candidate| MediaSource {
            title: langs.pick_owned(&candidate.title),
            id: candidate.id,
            is_default: candidate.default,
        })
        .collect();
    Json(MediaSources { sources }).into_response()
}

/// `GET /share-targets`：任何已加入的家人都能取，不看“管理成员”权限。
pub(super) async fn share_targets(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
) -> Response {
    let people = share_targets_of(&gateway, &caller.who);
    Json(ShareTargets { people }).into_response()
}

/// 能共享给谁：主人和已加入的家人，不含自己；主人在前，家人按用户名排。
/// 规则只写在这里，主人要改（比如加上好友）只改这一处。
fn share_targets_of(gateway: &Gateway, caller: &Person) -> Vec<ShareTarget> {
    let owner = gateway.owner.profile();
    let owner = (owner.username != caller.username).then(|| ShareTarget {
        user_id: matrix_user_id(gateway, &owner.username),
        display_name: owner.display_name,
    });
    let members = gateway
        .members
        .list()
        .into_iter()
        .filter(|record| {
            record.status == MemberStatus::Joined && record.username != caller.username
        })
        .map(|record| ShareTarget {
            user_id: matrix_user_id(gateway, &record.username),
            display_name: Some(record.display_name),
        });
    owner.into_iter().chain(members).collect()
}
