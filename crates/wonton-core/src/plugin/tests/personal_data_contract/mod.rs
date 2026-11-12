//! `personal_data` 能力的契约测试（ADR-0098 第 5 节）：同一组用例跑在两个互不相干的假提供方上。
//!
//! 用例只用契约里写的东西——路径、`X-Wonton-User`、按使用方的凭据、JSON 形状——断言两边的结果一样，
//! 换提供方时核心不用改。非 2xx 一律回 [`PersonalDataProblem`]，不认识的取值落 `Other`。
//! 某个提供方按用途换了上游也能过这组用例（假提供方一个按索引存，一个从头扫）。

use std::time::Duration;

use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::{
    CLEANUP_PATH, CleanupResult, DATA_PATH, DataQueryResult, EXPORT_PATH, HEALTH_PATH,
    HealthSample, HealthSamples, HealthValue, LOCATION_PATH, LocationPoint, LocationPointKind,
    PersonalDataFailure, PersonalDataProblem, QUERY_PATH, RetentionPolicy, RetentionSetting,
    SleepStage, WONTON_USER_HEADER,
};

use super::super::personal_data::PersonalDataClient;
use super::super::upstreams::Endpoint;
use crate::http::HttpClient;

mod map;
mod scan;
mod serve;

const ALICE: &str = "@alice:example.org";
const BOB: &str = "@bob:example.org";
const DAY_MS: u64 = 86_400_000;
const HOUR_MS: u64 = 3_600_000;

fn point(tst: u64, tid: &str) -> LocationPoint {
    LocationPoint {
        kind: LocationPointKind::Location,
        lat: 39.904_2,
        lon: 116.407_4,
        acc: 12.0,
        tst,
        tid: tid.into(),
    }
}

fn steps(start_ms: u64, source_id: &str) -> HealthSample {
    HealthSample {
        type_name: "steps".into(),
        start_ms,
        end_ms: None,
        value: HealthValue::Quantity {
            value: 1_200.0,
            unit: "count".into(),
        },
        source_id: source_id.into(),
    }
}

fn sample(type_name: &str, start_ms: u64, source_id: &str) -> HealthSample {
    HealthSample {
        type_name: type_name.into(),
        start_ms,
        end_ms: None,
        value: HealthValue::Quantity {
            value: 70.0,
            unit: "count/min".into(),
        },
        source_id: source_id.into(),
    }
}

fn batch(samples: Vec<HealthSample>, synced_through_ms: Option<u64>) -> HealthSamples {
    HealthSamples {
        samples,
        synced_through_ms,
    }
}

fn retained(type_name: &str, policy: RetentionPolicy) -> RetentionSetting {
    RetentionSetting {
        type_name: type_name.into(),
        retention: policy,
    }
}

fn healthy(
    result: Result<DataQueryResult, PersonalDataProblem>,
) -> (Vec<HealthSample>, Option<u64>) {
    match result {
        Ok(DataQueryResult::Health {
            samples,
            synced_through_ms,
        }) => (samples, synced_through_ms),
        other => panic!("应当是健康形状：{other:?}"),
    }
}

/// 原始请求：用来扮演比提供方新的核心，写它还不认识的取值，或者故意不带数据主体。
async fn raw(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    method: Method,
    path: &str,
    user: Option<&str>,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    let mut request = http
        .request(method, endpoint.url(path))
        .header("authorization", format!("Bearer {}", serve::KEY));
    if let Some(user) = user {
        request = request.header(WONTON_USER_HEADER, user);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// 收点：同一个设备的同一个测量时间重传只留一份，回查按测量时间从早到晚；不认识的 `_type` 落 `Other`。
async fn points(
    client: &PersonalDataClient,
    http: &reqwest::Client,
    endpoint: &Endpoint,
    now: u64,
) {
    let now_s = now / 1000;
    let first = point(now_s - HOUR_MS / 1000, "phone");
    let second = point(now_s, "phone");
    for point in [&first, &second, &first] {
        assert_eq!(
            client.write_location(ALICE, point).await.unwrap().accepted,
            1
        );
    }
    let DataQueryResult::Location { points } = client
        .query(ALICE, "location", now - 2 * HOUR_MS, now + 2 * HOUR_MS)
        .await
        .unwrap()
    else {
        panic!("位置查询应当是位置形状");
    };
    assert_eq!(points.len(), 2);
    assert_eq!(points[0].tst, first.tst);
    assert_eq!(points[0].tid, "phone");
    assert_eq!(points[0].acc, 12.0);
    assert_eq!(points[1].tst, second.tst);

    // 比提供方新的位置写法：不认识 `_type` 落 `Other`，多余字段原样留着（导出里核对）。
    let (status, _) = raw(
        http,
        endpoint,
        Method::POST,
        LOCATION_PATH,
        Some(ALICE),
        Some(json!({
            "_type": "waypoint",
            "lat": 39.9,
            "lon": 116.4,
            "acc": 20.0,
            "tst": now_s + 600,
            "tid": "phone",
            "batt": 88,
        })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // 测量时间是秒，查询是毫秒：从下一整秒起查，免得把上一秒的点也带上。
    let after = (now_s + 1) * 1000;
    let DataQueryResult::Location { points } = client
        .query(ALICE, "location", after, after + 2 * HOUR_MS)
        .await
        .unwrap()
    else {
        panic!("位置查询应当是位置形状");
    };
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].kind, LocationPointKind::Other);
}

/// 收样本：批里的同步时刻记下来，同一个来源重传只留一份；不认识的睡眠阶段落 `Other`。
async fn health(
    client: &PersonalDataClient,
    http: &reqwest::Client,
    endpoint: &Endpoint,
    now: u64,
) {
    let samples = batch(
        vec![
            steps(now - 60 * DAY_MS, "steps-old"),
            steps(now - 5 * DAY_MS, "steps-new"),
        ],
        Some(now - 1000),
    );
    assert_eq!(
        client.write_health(ALICE, &samples).await.unwrap().accepted,
        2
    );
    assert_eq!(
        client
            .write_health(
                ALICE,
                &batch(vec![steps(now - 5 * DAY_MS, "steps-new")], Some(now - 1000))
            )
            .await
            .unwrap()
            .accepted,
        1
    );
    let (samples, synced) = healthy(client.query(ALICE, "steps", now - 90 * DAY_MS, now).await);
    assert_eq!(samples.len(), 2);
    assert_eq!(synced, Some(now - 1000));

    let (status, _) = raw(
        http,
        endpoint,
        Method::POST,
        HEALTH_PATH,
        Some(ALICE),
        Some(json!({
            "samples": [{
                "type_name": "sleep",
                "start_ms": now - 3 * DAY_MS,
                "end_ms": now - 3 * DAY_MS + HOUR_MS,
                "value": {"kind": "sleep", "stage": "awake_asleep"},
                "source_id": "sleep-novel",
            }],
            "synced_through_ms": now - 3 * DAY_MS,
        })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let (samples, _) = healthy(client.query(ALICE, "sleep", now - 4 * DAY_MS, now).await);
    assert_eq!(
        samples[0].value,
        HealthValue::Sleep {
            stage: SleepStage::Other
        }
    );

    // 比核心新的类型照健康类型收下、原样回查。
    assert_eq!(
        client
            .write_health(
                ALICE,
                &batch(vec![sample("gait_speed", now, "gait-1")], Some(now))
            )
            .await
            .unwrap()
            .accepted,
        1
    );
    let (samples, _) = healthy(
        client
            .query(ALICE, "gait_speed", now - HOUR_MS, now + HOUR_MS)
            .await,
    );
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].type_name, "gait_speed");

    // 一条心率样本留着和清理对照。
    client
        .write_health(
            ALICE,
            &batch(vec![sample("heart_rate", now, "heart-1")], Some(now)),
        )
        .await
        .unwrap();
}

/// 按人分开：鲍勃看不到爱丽丝的数据，导出也只有自己的。
async fn exports_are_per_person(
    client: &PersonalDataClient,
    http: &reqwest::Client,
    endpoint: &Endpoint,
    now: u64,
) {
    let (samples, _) = healthy(client.query(BOB, "steps", now - 90 * DAY_MS, now).await);
    assert!(samples.is_empty());
    let exported = client.export(ALICE).await.unwrap();
    assert_eq!(exported.location.len(), 3);
    assert_eq!(exported.health.len(), 5);
    assert!(exported.exported_at_ms >= now - 1000);
    let wire = raw(http, endpoint, Method::GET, EXPORT_PATH, Some(ALICE), None)
        .await
        .1;
    assert_eq!(wire["location"][2]["_type"], "waypoint");
    assert_eq!(wire["location"][2]["batt"], 88);
    let bob = client.export(BOB).await.unwrap();
    assert!(bob.location.is_empty());
    assert!(bob.health.is_empty());
}

/// 撤回时按类型按时间段删；保存期限到期删，`Never` 的收下就删，不认识的期限不提前删。
async fn delete_and_retention(client: &PersonalDataClient, now: u64) {
    let CleanupResult { deleted } = client
        .delete(ALICE, "steps", now - 61 * DAY_MS, now - 59 * DAY_MS)
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    let (samples, _) = healthy(client.query(ALICE, "steps", now - 90 * DAY_MS, now).await);
    assert_eq!(samples.len(), 1);

    let CleanupResult { deleted } = client
        .cleanup(
            ALICE,
            vec![
                retained("location", RetentionPolicy::Never),
                retained("steps", RetentionPolicy::Days { days: 30 }),
                retained("sleep", RetentionPolicy::Days { days: 1 }),
                retained("heart_rate", RetentionPolicy::Days { days: 7 }),
            ],
        )
        .await
        .unwrap();
    assert_eq!(deleted, 4);
    assert_eq!(
        client
            .write_location(ALICE, &point(now / 1000 + 20, "phone"))
            .await
            .unwrap()
            .accepted,
        0
    );
    let DataQueryResult::Location { points } = client
        .query(ALICE, "location", now, now + 2 * HOUR_MS)
        .await
        .unwrap()
    else {
        panic!("位置查询应当是位置形状");
    };
    assert!(points.is_empty());
    let (samples, _) = healthy(client.query(ALICE, "sleep", now - 4 * DAY_MS, now).await);
    assert!(samples.is_empty());
    let (samples, _) = healthy(
        client
            .query(ALICE, "heart_rate", now - HOUR_MS, now + HOUR_MS)
            .await,
    );
    assert_eq!(samples.len(), 1);

    // 比核心新的期限：按最长的处理，不提前删。
    let CleanupResult { deleted } = client
        .cleanup(ALICE, vec![retained("steps", RetentionPolicy::Other)])
        .await
        .unwrap();
    assert_eq!(deleted, 0);
    let (samples, _) = healthy(client.query(ALICE, "steps", now - 90 * DAY_MS, now).await);
    assert_eq!(samples.len(), 1);
}

/// 不存的类型、比核心新的形状和原因、凭据和请求头、写法不对的请求。
async fn failures(
    client: &PersonalDataClient,
    http: &reqwest::Client,
    endpoint: &Endpoint,
    now: u64,
) {
    let unsupported = client
        .query(ALICE, "vehicle_position", now, now)
        .await
        .unwrap_err();
    assert_eq!(unsupported.reason, PersonalDataFailure::NotSupported);
    assert!(unsupported.message.contains("vehicle_position"));
    match client.query(ALICE, "future_type", now, now).await {
        Ok(DataQueryResult::Other) => {}
        other => panic!("应当是不认识的形状：{other:?}"),
    }
    let novel = client
        .query(ALICE, "novel_failure", now, now)
        .await
        .unwrap_err();
    assert_eq!(novel.reason, PersonalDataFailure::Other);
    assert_eq!(novel.message, "手机和服务器的时间差太多");

    let wrong_key =
        PersonalDataClient::new(endpoint.clone(), "wrong".into(), HttpClient::new()).unwrap();
    let denied = wrong_key.query(ALICE, "steps", now, now).await.unwrap_err();
    assert_eq!(denied.reason, PersonalDataFailure::ProviderUnavailable);
    assert!(denied.message.contains("凭据"));
    let (status, missing) = raw(http, endpoint, Method::GET, QUERY_PATH, None, None).await;
    assert_eq!(status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(missing["reason"], "provider_unavailable");
    assert!(
        missing["message"]
            .as_str()
            .unwrap()
            .contains(WONTON_USER_HEADER)
    );

    let (_, malformed) = raw(
        http,
        endpoint,
        Method::POST,
        LOCATION_PATH,
        Some(ALICE),
        Some(json!({"lat": 1.0})),
    )
    .await;
    assert_eq!(malformed["reason"], "provider_unavailable");
}

/// 原始写法也能直接进，删除也走原始请求。
async fn raw_writes(
    client: &PersonalDataClient,
    http: &reqwest::Client,
    endpoint: &Endpoint,
    now: u64,
) {
    let (status, accepted) = raw(
        http,
        endpoint,
        Method::POST,
        LOCATION_PATH,
        Some(BOB),
        Some(json!({
            "_type": "location",
            "lat": 1.0,
            "lon": 2.0,
            "acc": 3.0,
            "tst": now / 1000,
            "tid": "tablet",
        })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(accepted["accepted"], 1);
    assert_eq!(client.export(BOB).await.unwrap().location.len(), 1);
    let (_, deleted) = raw(
        http,
        endpoint,
        Method::DELETE,
        &wonton_protocol::data_path("location", 0, u64::MAX),
        Some(BOB),
        None,
    )
    .await;
    assert_eq!(deleted["deleted"], 1);
    assert_eq!(DATA_PATH, "/data");
    assert_eq!(CLEANUP_PATH, "/cleanup");
}

async fn contract(endpoint: Endpoint) {
    let client =
        PersonalDataClient::new(endpoint.clone(), serve::KEY.into(), HttpClient::new()).unwrap();
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let now = serve::now_ms();
    points(&client, &http, &endpoint, now).await;
    health(&client, &http, &endpoint, now).await;
    exports_are_per_person(&client, &http, &endpoint, now).await;
    delete_and_retention(&client, now).await;
    failures(&client, &http, &endpoint, now).await;
    raw_writes(&client, &http, &endpoint, now).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_indexed_provider_satisfies_the_contract() {
    let (endpoint, _served) = serve::start(map::MapStore::new()).await;
    contract(endpoint).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scanning_provider_satisfies_the_contract() {
    let (endpoint, _served) = serve::start(scan::ScanStore::new()).await;
    contract(endpoint).await;
}

/// 提供方在宿主网络时不能连 TCP，客户端要经它的 unix 套接字送请求，数据主体和凭据照带。
#[tokio::test]
async fn a_host_network_provider_is_reached_over_its_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("store.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let serving = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(
                    |request: hyper::Request<hyper::body::Incoming>| async move {
                        assert_eq!(request.uri().path(), QUERY_PATH);
                        assert_eq!(request.headers()[WONTON_USER_HEADER], ALICE);
                        assert_eq!(
                            request.headers()["authorization"],
                            format!("Bearer {}", serve::KEY)
                        );
                        Ok::<_, std::convert::Infallible>(hyper::Response::new(
                            axum::body::Body::from(
                                r#"{"kind":"health","samples":[],"synced_through_ms":7}"#,
                            ),
                        ))
                    },
                );
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await
                    .unwrap();
            });
        }
    });
    let client = PersonalDataClient::new(
        Endpoint::unix(&socket),
        serve::KEY.into(),
        HttpClient::new(),
    )
    .unwrap();
    let (samples, synced) = healthy(client.query(ALICE, "steps", 0, 10).await);
    assert!(samples.is_empty());
    assert_eq!(synced, Some(7));
    serving.abort();
}
