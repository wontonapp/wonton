use std::collections::BTreeMap;

use reqwest::StatusCode;
use serde_json::json;

use super::{Bed, ProviderAction, langs};

pub(super) async fn configure(bed: &Bed) {
    let initial = bed.configured().await;
    assert!(
        initial.is_empty(),
        "each adapter starts without configured providers"
    );
    for (key, expected) in [
        ("contract-key", None),
        ("rejected-key", Some("invalid_input")),
    ] {
        let result = bed
            .manager
            .custom_model_provider(&bed.custom(key), ProviderAction::Test, &langs())
            .await;
        match expected {
            None => result.unwrap(),
            Some(kind) => assert_eq!(result.unwrap_err().kind(), kind),
        }
        assert_eq!(
            bed.configured().await,
            initial,
            "test must not save its candidate"
        );
    }
    let mut unavailable = bed.custom("contract-key");
    unavailable.base_url = unavailable.base_url.replace("/v1", "/down/v1");
    let error = bed
        .manager
        .custom_model_provider(&unavailable, ProviderAction::Test, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "upstream_unavailable");
    assert_eq!(bed.configured().await, initial);
    let error = bed
        .manager
        .custom_model_provider(&bed.custom("rejected-key"), ProviderAction::Save, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(
        bed.configured().await,
        initial,
        "rejected saves must not persist"
    );
    bed.manager
        .custom_model_provider(&bed.custom("contract-key"), ProviderAction::Save, &langs())
        .await
        .unwrap();
    let configured = bed.configured().await;
    assert_eq!(configured.len(), 1);
    let id = &configured[0];
    bed.manager
        .model_provider_key(id, "contract-key", ProviderAction::Test, &langs())
        .await
        .unwrap();
    let error = bed
        .manager
        .model_provider_key(id, "rejected-key", ProviderAction::Test, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(bed.configured().await, configured);
    let before = bed.target.docker.execs().len();
    let error = bed
        .manager
        .model_provider_key(
            "not-a-provider",
            "contract-key",
            ProviderAction::Save,
            &langs(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(
        bed.target.docker.execs()[before..]
            .iter()
            .all(|call| !call.cmd.iter().any(|arg| arg == "not-a-provider"))
    );
}

pub(super) async fn finish(bed: &Bed, consumer: &BTreeMap<String, String>) {
    let configured = bed.configured().await;
    let output = bed
        .target
        .engine
        .exec_output(
            "contract-provider",
            &bed.target.spec.providers.as_ref().unwrap().list,
            None,
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap();
    assert_eq!(output.exit_code, 0);
    let list: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
    let ids: Vec<_> = list["presets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(configured.iter().all(|id| ids.contains(&id.as_str())));
    let preset = ids
        .into_iter()
        .find(|id| !configured.iter().any(|saved| saved == id))
        .unwrap();
    let error = bed
        .manager
        .model_provider_key(preset, "contract-key", ProviderAction::Test, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "upstream_unavailable");
    assert_eq!(
        bed.configured().await,
        configured,
        "preset testing is isolated too"
    );
    let error = bed
        .manager
        .model_provider_key(preset, "contract-key", ProviderAction::Save, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "upstream_unavailable");
    assert!(
        bed.configured().await.iter().any(|id| id == preset),
        "disconnected save keeps the supplied key"
    );
    bed.manager
        .custom_model_provider(&bed.custom("rotated-key"), ProviderAction::Save, &langs())
        .await
        .unwrap();
    super::consumer::wait_for_reply(bed, consumer, "rotated reply").await;
    let error = bed
        .manager
        .custom_model_provider(&bed.custom("rejected-key"), ProviderAction::Save, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    let response = bed.chat(consumer, json!({"model":consumer["WONTON_MODEL_DEFAULT"],"messages":[{"role":"user","content":"still works"}],"stream":false})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["choices"][0]["message"]["content"],
        "rotated reply"
    );
    for call in bed.target.docker.execs() {
        for secret in [
            "contract-key",
            "rotated-key",
            "rejected-key",
            &consumer["WONTON_MODEL_API_KEY"],
        ] {
            assert!(
                !call.cmd.iter().any(|arg| arg.contains(secret)),
                "secrets belong on stdin, not argv"
            );
        }
    }
}
