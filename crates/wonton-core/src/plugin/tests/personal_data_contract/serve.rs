//! 两个假提供方共用的 HTTP 外壳：校验凭据和 `X-Wonton-User`、读写 JSON、把失败写成 [`PersonalDataProblem`]。
//! 存储语义在各自的 [`Store`] 里，外壳不碰数据，两份实现只有下面这个 trait 相同。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wonton_protocol::{
    CLEANUP_PATH, CleanupRequest, DATA_PATH, EXPORT_PATH, HEALTH_PATH, HealthSamples,
    LOCATION_PATH, LocationPoint, PersonalDataFailure, PersonalDataProblem, QUERY_PATH,
    RetentionPolicy, WONTON_USER_HEADER,
};

use super::super::super::upstreams::Endpoint;

/// 契约测试用的凭据：核心按使用方签发（ADR-0053），两种提供方都只认它。
pub(super) const KEY: &str = "contract-key";

/// 提供方的回话：成功的 JSON，或者一项失败和原文。
pub(super) type Reply = Result<Value, PersonalDataProblem>;

/// 一次查询或删除的范围，两端闭区间。
pub(super) struct Ask {
    pub type_name: String,
    pub from_ms: u64,
    pub to_ms: u64,
}

pub(super) trait Store: Send + 'static {
    fn write_location(&mut self, user: &str, point: Value) -> Reply;
    fn write_health(&mut self, user: &str, samples: Value) -> Reply;
    fn query(&self, user: &str, ask: &Ask) -> Reply;
    fn delete(&mut self, user: &str, ask: &Ask) -> Reply;
    fn export(&self, user: &str) -> Reply;
    fn cleanup(&mut self, user: &str, request: Value) -> Reply;
}

pub(super) struct Served(tokio::task::JoinHandle<()>);

impl Drop for Served {
    fn drop(&mut self) {
        self.0.abort();
    }
}

type Shared = Arc<Mutex<dyn Store>>;

pub(super) async fn start(store: impl Store) -> (Endpoint, Served) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let shared: Shared = Arc::new(Mutex::new(store));
    let serving = tokio::spawn(async move {
        axum::serve(listener, router(shared)).await.unwrap();
    });
    (endpoint, Served(serving))
}

/// 合法的 HTTP 错误体：`{"reason": …, "message": …}`。`novel_failure` 用核心不认识的原因，
/// 验证客户端把不认识的取值落成 [`PersonalDataFailure::Other`]。
pub(super) fn problem(reason: PersonalDataFailure, message: &str) -> PersonalDataProblem {
    PersonalDataProblem {
        reason,
        message: message.to_owned(),
    }
}

pub(super) fn invalid(message: impl Into<String>) -> PersonalDataProblem {
    problem(PersonalDataFailure::ProviderUnavailable, &message.into())
}

fn router(shared: Shared) -> Router {
    Router::new()
        .route(LOCATION_PATH, post(location))
        .route(HEALTH_PATH, post(health))
        .route(QUERY_PATH, get(query))
        .route(DATA_PATH, delete(remove))
        .route(EXPORT_PATH, get(export))
        .route(CLEANUP_PATH, post(cleanup))
        .with_state(shared)
}

/// 校验过的调用方：哪个是数据主体，以及凭据对不对。
struct Caller {
    user: String,
}

impl FromRequestParts<Shared> for Caller {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _: &Shared) -> Result<Self, Self::Rejection> {
        let authorized = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some(&format!("Bearer {KEY}"));
        if !authorized {
            return Err(reject(problem(
                PersonalDataFailure::ProviderUnavailable,
                "凭据不对",
            )));
        }
        let user = parts
            .headers
            .get(WONTON_USER_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|user| !user.is_empty())
            .ok_or_else(|| {
                reject(problem(
                    PersonalDataFailure::ProviderUnavailable,
                    "缺 X-Wonton-User",
                ))
            })?;
        Ok(Self {
            user: user.to_owned(),
        })
    }
}

#[derive(Deserialize)]
struct WireAsk {
    #[serde(rename = "type")]
    type_name: String,
    from: u64,
    to: u64,
}

async fn location(
    State(shared): State<Shared>,
    caller: Caller,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(e) => return reject(invalid(format!("请求体不是 JSON：{e}"))),
    };
    reply(shared.lock().unwrap().write_location(&caller.user, body))
}

async fn health(
    State(shared): State<Shared>,
    caller: Caller,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(e) => return reject(invalid(format!("请求体不是 JSON：{e}"))),
    };
    reply(shared.lock().unwrap().write_health(&caller.user, body))
}

async fn query(
    State(shared): State<Shared>,
    caller: Caller,
    ask: Result<Query<WireAsk>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Query(ask) = match ask {
        Ok(ask) => ask,
        Err(e) => return reject(invalid(format!("查询串不对：{e}"))),
    };
    match ask.type_name.as_str() {
        // 比核心新的提供方给的形状：客户端要落成 `DataQueryResult::Other`。
        "future_type" => {
            return (StatusCode::OK, Json(json!({"kind": "ledger_digest"}))).into_response();
        }
        // 比核心新的失败原因：客户端要落成 `PersonalDataFailure::Other`，原文照带。
        "novel_failure" => {
            return (
                StatusCode::CONFLICT,
                Json(json!({"reason": "clock_skew", "message": "手机和服务器的时间差太多"})),
            )
                .into_response();
        }
        _ => {}
    }
    let ask = Ask {
        type_name: ask.type_name,
        from_ms: ask.from,
        to_ms: ask.to,
    };
    reply(shared.lock().unwrap().query(&caller.user, &ask))
}

async fn remove(
    State(shared): State<Shared>,
    caller: Caller,
    ask: Result<Query<WireAsk>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Query(ask) = match ask {
        Ok(ask) => ask,
        Err(e) => return reject(invalid(format!("查询串不对：{e}"))),
    };
    let ask = Ask {
        type_name: ask.type_name,
        from_ms: ask.from,
        to_ms: ask.to,
    };
    reply(shared.lock().unwrap().delete(&caller.user, &ask))
}

async fn export(State(shared): State<Shared>, caller: Caller) -> Response {
    reply(shared.lock().unwrap().export(&caller.user))
}

async fn cleanup(
    State(shared): State<Shared>,
    caller: Caller,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(e) => return reject(invalid(format!("请求体不是 JSON：{e}"))),
    };
    reply(shared.lock().unwrap().cleanup(&caller.user, body))
}

fn reply(result: Reply) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(problem) => reject(problem),
    }
}

fn reject(problem: PersonalDataProblem) -> Response {
    let status = match problem.reason {
        PersonalDataFailure::PermissionDenied => StatusCode::FORBIDDEN,
        PersonalDataFailure::NotSupported => StatusCode::UNPROCESSABLE_ENTITY,
        PersonalDataFailure::ProviderUnavailable => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(problem)).into_response()
}

/// 现在的 Unix 毫秒，清理和导出的时间基准。
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// 用户表，两种实现都按人存，只是存法和查法不同。
pub(super) fn user<'a, T: Default>(users: &'a mut BTreeMap<String, T>, user: &str) -> &'a mut T {
    users.entry(user.to_owned()).or_default()
}

/// 核心定的健康类型名，供“这个类型认不认”判断；比核心新的类型由提供方照健康类型收下。
pub(super) const HEALTH_TYPES: [&str; 7] = [
    "steps",
    "walking_running_distance",
    "active_energy",
    "heart_rate",
    "resting_heart_rate",
    "sleep",
    "body_weight",
];

const DAY_MS: u64 = 86_400_000;

/// 一个位置点的索引字段；`wire` 是原样的 JSON，回查时原样回，新的 `_type` 和多余字段都不丢。
pub(super) struct StoredPoint {
    pub tst: u64,
    pub tid: String,
    pub wire: Value,
}

/// 校验一个位置点，留下去重和排序要的字段。
pub(super) fn point(wire: Value) -> Result<StoredPoint, PersonalDataProblem> {
    let parsed: LocationPoint =
        serde_json::from_value(wire.clone()).map_err(|e| invalid(format!("位置点不对：{e}")))?;
    Ok(StoredPoint {
        tst: parsed.tst,
        tid: parsed.tid,
        wire,
    })
}

/// 一个健康样本的索引字段；`wire` 同上。
pub(super) struct StoredSample {
    pub type_name: String,
    pub start_ms: u64,
    pub source_id: String,
    pub wire: Value,
}

pub(super) struct Batch {
    pub samples: Vec<StoredSample>,
    pub synced_through_ms: Option<u64>,
}

/// 校验一批健康样本。
pub(super) fn batch(wire: Value) -> Result<Batch, PersonalDataProblem> {
    let parsed: HealthSamples =
        serde_json::from_value(wire.clone()).map_err(|e| invalid(format!("样本批次不对：{e}")))?;
    let raw = wire["samples"].as_array().cloned().unwrap_or_default();
    let samples = parsed
        .samples
        .into_iter()
        .zip(raw)
        .map(|(sample, wire)| StoredSample {
            type_name: sample.type_name,
            start_ms: sample.start_ms,
            source_id: sample.source_id,
            wire,
        })
        .collect();
    Ok(Batch {
        samples,
        synced_through_ms: parsed.synced_through_ms,
    })
}

/// 这次清理对每个类型怎么处理。
pub(super) fn policies(
    wire: &Value,
) -> Result<Vec<(String, RetentionPolicy)>, PersonalDataProblem> {
    let request: CleanupRequest =
        serde_json::from_value(wire.clone()).map_err(|e| invalid(format!("清理请求不对：{e}")))?;
    Ok(request
        .policies
        .into_iter()
        .map(|setting| (setting.type_name, setting.retention))
        .collect())
}

/// 写入时要不要留：`Never` 的类型收下就丢，其余先存下，到期由清理删。
pub(super) fn keeps(policy: Option<&RetentionPolicy>) -> bool {
    !matches!(policy, Some(RetentionPolicy::Never))
}

/// 清理时删早于哪一刻；`None` 是不删（不认识的期限按最长的处理）。
pub(super) fn cutoff(policy: &RetentionPolicy) -> Option<u64> {
    match policy {
        RetentionPolicy::Never => Some(u64::MAX),
        RetentionPolicy::Days { days } => Some(now_ms().saturating_sub(*days as u64 * DAY_MS)),
        RetentionPolicy::Other => None,
    }
}
