use super::*;

fn decisions() -> serde_json::Value {
    json!({
        "source": "github:example/wonton-consumer@v1.0.0",
        "digest": "sha256:consumer",
        "plan": "consumer-plan",
        "scope": {"type": "bots", "bots": [{"plugin_id": "ai.nous.hermes", "bot": "alice"}]},
        "grant": ["models"],
        "enabled_permissions": ["storage.share.documents:read"],
        "secrets": {"api_key": "consumer-secret"},
        "devices": ["consumer-usb"],
        "lan_webui": false,
        "install_together": [{
            "id": "app.provider",
            "request": {
                "source": "github:example/wonton-provider@v2.0.0",
                "digest": "sha256:provider",
                "plan": "provider-plan",
                "scope": {"type": "all"},
                "grant": ["network.host"],
                "enabled_permissions": ["docker.socket"],
                "secrets": {"api_key": "provider-secret"},
                "devices": ["provider-serial"],
                "lan_webui": true,
                "install_together": [],
                "connect_plugins": []
            }
        }],
        "connect_plugins": ["app.existing-consumer"]
    })
}

#[test]
fn consumer_and_provider_keep_independent_install_decisions() {
    let wire = decisions();
    let request: InstallRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);

    let decoded: InstallRequest = round_trip(&request);
    assert_eq!(decoded, request);
    assert_eq!(decoded.secrets["api_key"].expose(), "consumer-secret");
    let provider = &decoded.install_together[0].request;
    assert_eq!(provider.secrets["api_key"].expose(), "provider-secret");
    assert!(provider.install_together.is_empty());
    assert!(provider.connect_plugins.is_empty());
    assert_eq!(decoded.connect_plugins, ["app.existing-consumer"]);
}

#[test]
fn provider_requests_reject_both_kinds_of_nested_decisions() {
    for (field, nested) in [
        (
            "install_together",
            json!([{
                "id": "app.nested-provider",
                "request": {
                    "source": "github:example/wonton-nested@v3.0.0",
                    "digest": "sha256:nested",
                    "grant": []
                }
            }]),
        ),
        ("connect_plugins", json!(["app.nested-consumer"])),
    ] {
        let mut wire = decisions();
        wire["install_together"][0]["request"][field] = nested;
        assert!(serde_json::from_value::<InstallRequest>(wire).is_err());
    }
}

#[test]
fn omitted_provider_request_defaults_but_required_request_fields_do_not() {
    let provider: InstallTogether = serde_json::from_value(json!({"id": "app.provider"})).unwrap();
    assert_eq!(provider.request, InstallRequest::default());

    for required in ["source", "digest", "grant"] {
        let mut wire = decisions();
        wire.as_object_mut().unwrap().remove(required);
        assert!(serde_json::from_value::<InstallRequest>(wire).is_err());
    }
}

#[test]
fn plans_are_opaque_and_round_trip_exactly() {
    let plan = " \0opaque:\n{\"future\":true} 计划 ";
    let preview = InstallPreview {
        plan: plan.into(),
        ..preview()
    };
    let mut request: InstallRequest = serde_json::from_value(decisions()).unwrap();
    request.plan = plan.into();

    assert_eq!(serde_json::to_value(&preview).unwrap()["plan"], json!(plan));
    assert_eq!(round_trip(&preview).plan, plan);
    assert_eq!(serde_json::to_value(&request).unwrap()["plan"], json!(plan));
    assert_eq!(round_trip(&request).plan, plan);
}

#[test]
fn omitted_preview_and_request_plans_default_to_empty() {
    let mut preview_wire = serde_json::to_value(preview()).unwrap();
    preview_wire.as_object_mut().unwrap().remove("plan");
    let preview: InstallPreview = serde_json::from_value(preview_wire).unwrap();
    assert!(preview.plan.is_empty());

    let mut request_wire = decisions();
    request_wire.as_object_mut().unwrap().remove("plan");
    request_wire["install_together"][0]["request"]
        .as_object_mut()
        .unwrap()
        .remove("plan");
    let request: InstallRequest = serde_json::from_value(request_wire).unwrap();
    assert!(request.plan.is_empty());
    assert!(request.install_together[0].request.plan.is_empty());
}
