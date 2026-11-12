use serde_json::json;

use super::*;

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.hermes".into(),
        bot: name.into(),
    }
}

#[test]
fn type_names_are_the_wire_names_and_carry_their_category() {
    let location = json!(PersonalDataType::Location);
    assert_eq!(location, json!("location"));
    assert_eq!(
        PersonalDataType::WalkingRunningDistance.as_str(),
        "walking_running_distance"
    );
    assert_eq!(
        PersonalDataType::HeartRate.category(),
        PersonalDataCategory::Health
    );
    assert_eq!(
        PersonalDataType::Sleep.category(),
        PersonalDataCategory::Health
    );
    assert_eq!(
        PersonalDataType::Location.category(),
        PersonalDataCategory::Location
    );
    for data_type in [
        PersonalDataType::Location,
        PersonalDataType::Steps,
        PersonalDataType::WalkingRunningDistance,
        PersonalDataType::ActiveEnergy,
        PersonalDataType::HeartRate,
        PersonalDataType::RestingHeartRate,
        PersonalDataType::Sleep,
        PersonalDataType::BodyWeight,
    ] {
        assert_eq!(
            serde_json::from_value::<PersonalDataType>(json!(data_type.as_str())).unwrap(),
            data_type
        );
    }
}

#[test]
fn a_location_point_round_trips_in_the_owntracks_writing() {
    let point = LocationPoint {
        kind: LocationPointKind::Location,
        lat: 39.904_2,
        lon: 116.407_4,
        acc: 12.0,
        tst: 1_762_500_000,
        tid: "phone-hermes".into(),
    };
    let written = json!({
        "_type": "location",
        "lat": 39.904_2,
        "lon": 116.407_4,
        "acc": 12.0,
        "tst": 1_762_500_000u64,
        "tid": "phone-hermes",
    });
    assert_eq!(json!(point), written);
    assert_eq!(
        serde_json::from_value::<LocationPoint>(written).unwrap(),
        point
    );
}

#[test]
fn health_samples_round_trip_a_quantity_and_a_sleep_stage() {
    let samples = HealthSamples {
        samples: vec![
            HealthSample {
                type_name: "steps".into(),
                start_ms: 1_762_500_000_000,
                end_ms: None,
                value: HealthValue::Quantity {
                    value: 1200.0,
                    unit: "count".into(),
                },
                source_id: "s-1".into(),
            },
            HealthSample {
                type_name: "sleep".into(),
                start_ms: 1_762_500_000_000,
                end_ms: Some(1_762_503_600_000),
                value: HealthValue::Sleep {
                    stage: SleepStage::Deep,
                },
                source_id: "s-2".into(),
            },
        ],
        synced_through_ms: Some(1_762_510_000_000),
    };
    assert_eq!(
        json!(samples),
        json!({
            "samples": [
                {
                    "type_name": "steps",
                    "start_ms": 1_762_500_000_000u64,
                    "end_ms": null,
                    "value": {"kind": "quantity", "value": 1200.0, "unit": "count"},
                    "source_id": "s-1",
                },
                {
                    "type_name": "sleep",
                    "start_ms": 1_762_500_000_000u64,
                    "end_ms": 1_762_503_600_000u64,
                    "value": {"kind": "sleep", "stage": "deep"},
                    "source_id": "s-2",
                },
            ],
            "synced_through_ms": 1_762_510_000_000u64,
        })
    );
    assert_eq!(
        serde_json::from_value::<HealthSamples>(json!(samples)).unwrap(),
        samples
    );
}

#[test]
fn unknown_values_fall_back_to_other() {
    assert_eq!(
        serde_json::from_value::<PersonalDataType>(json!("blood_oxygen")).unwrap(),
        PersonalDataType::Other
    );
    assert_eq!(
        serde_json::from_value::<PersonalDataCategory>(json!("screen_time")).unwrap(),
        PersonalDataCategory::Other
    );
    assert_eq!(
        serde_json::from_value::<HealthValue>(json!({"kind": "category", "code": 7})).unwrap(),
        HealthValue::Other
    );
    assert_eq!(
        serde_json::from_value::<SleepStage>(json!("asleep_light")).unwrap(),
        SleepStage::Other
    );
    assert_eq!(
        serde_json::from_value::<RetentionPolicy>(json!({"policy": "forever"})).unwrap(),
        RetentionPolicy::Other
    );
    assert_eq!(
        serde_json::from_value::<DataQueryResult>(json!({"kind": "summary"})).unwrap(),
        DataQueryResult::Other
    );
}

#[test]
fn failures_round_trip_and_an_unknown_one_falls_back() {
    for failure in [
        PersonalDataFailure::PermissionDenied,
        PersonalDataFailure::OsPermissionMissing,
        PersonalDataFailure::DeviceUnreachable,
        PersonalDataFailure::DeviceNoResponse,
        PersonalDataFailure::DeviceLocked,
        PersonalDataFailure::RateLimited,
        PersonalDataFailure::NotSupported,
        PersonalDataFailure::ProviderUnavailable,
    ] {
        let written = json!(failure);
        assert_eq!(
            serde_json::from_value::<PersonalDataFailure>(written).unwrap(),
            failure
        );
    }
    assert_eq!(
        serde_json::from_value::<PersonalDataFailure>(json!("quota_exhausted")).unwrap(),
        PersonalDataFailure::Other
    );
    let problem = PersonalDataProblem {
        reason: PersonalDataFailure::RateLimited,
        message: "每小时 12 次，10:24 之后能再查".into(),
    };
    assert_eq!(
        serde_json::from_value::<PersonalDataProblem>(json!({
            "reason": "rate_limited",
            "message": "每小时 12 次，10:24 之后能再查",
        }))
        .unwrap(),
        problem
    );
}

#[test]
fn the_page_round_trips_a_row_with_a_grant_and_a_read() {
    let view = PersonalDataViewModel {
        loading: false,
        problem: None,
        types: vec![PersonalDataTypeRow {
            data_type: PersonalDataType::Steps,
            enabled: true,
            supported: true,
            location_scope: None,
            os_permission: OsPermissionState::Unknown,
            retention: RetentionPolicy::Days { days: 365 },
            disclosure_acknowledged: true,
            model_disclosure_acknowledged: Some(true),
            bots: vec![PersonalDataBotGrant {
                bot: bot("helper"),
                enabled: true,
                model_service: Some("本地模型".into()),
            }],
            synced_through_ms: Some(1_762_500_000_000),
            stale: false,
            recent_reads: vec![PersonalDataRead {
                bot: bot("helper"),
                at_ms: 1_762_500_000_000,
            }],
        }],
    };
    assert_eq!(json!(view)["types"][0]["bots"][0]["bot"]["bot"], "helper");
    assert_eq!(
        json!(view)["types"][0]["recent_reads"][0]["at_ms"],
        1_762_500_000_000u64
    );
    assert_eq!(
        serde_json::from_value::<PersonalDataViewModel>(json!(view)).unwrap(),
        view
    );
}

/// 契约：核心比 App 新时，App 不显示不认识的类型，也不把新期限当成已到期的删掉。
#[test]
fn an_unknown_row_type_parses() {
    let row = serde_json::from_value::<PersonalDataTypeRow>(json!({
        "data_type": "blood_oxygen",
        "enabled": false,
        "supported": true,
        "location_scope": null,
        "os_permission": "unknown",
        "retention": {"policy": "forever"},
        "disclosure_acknowledged": false,
        "model_disclosure_acknowledged": null,
        "bots": [],
        "synced_through_ms": null,
        "stale": false,
        "recent_reads": [],
    }))
    .unwrap();
    assert_eq!(row.data_type, PersonalDataType::Other);
    assert_eq!(row.retention, RetentionPolicy::Other);
}

#[test]
fn paths_carry_the_type_and_the_closed_window() {
    assert_eq!(
        query_path("heart_rate", 1_762_500_000_000, 1_762_503_600_000),
        "/query?type=heart_rate&from=1762500000000&to=1762503600000"
    );
    assert_eq!(
        data_path("location", 0, 99),
        "/data?type=location&from=0&to=99"
    );
    assert_eq!(QUERY_PATH, "/query");
    assert_eq!(DATA_PATH, "/data");
    assert_eq!(FAILURE_META, "app.wonton/failure");
}

#[test]
fn query_results_round_trip_both_shapes() {
    let points = DataQueryResult::Location {
        points: vec![LocationPoint {
            kind: LocationPointKind::Location,
            lat: 1.0,
            lon: 2.0,
            acc: 3.0,
            tst: 4,
            tid: "phone".into(),
        }],
    };
    let health = DataQueryResult::Health {
        samples: vec![],
        synced_through_ms: None,
    };
    assert_eq!(
        serde_json::from_value::<DataQueryResult>(json!(points)).unwrap(),
        points
    );
    assert_eq!(
        serde_json::from_value::<DataQueryResult>(json!(health)).unwrap(),
        health
    );
}

#[test]
fn a_cleanup_request_round_trips_policies() {
    let request = CleanupRequest {
        policies: vec![
            RetentionSetting {
                type_name: "location".into(),
                retention: RetentionPolicy::Days { days: 30 },
            },
            RetentionSetting {
                type_name: "steps".into(),
                retention: RetentionPolicy::Never,
            },
        ],
    };
    assert_eq!(
        json!(request),
        json!({
            "policies": [
                {"type_name": "location", "retention": {"policy": "days", "days": 30}},
                {"type_name": "steps", "retention": {"policy": "never"}},
            ],
        })
    );
    assert_eq!(
        serde_json::from_value::<CleanupRequest>(json!(request)).unwrap(),
        request
    );
    assert_eq!(
        serde_json::from_value::<CleanupResult>(json!({"deleted": 3})).unwrap(),
        CleanupResult { deleted: 3 }
    );
}
