use super::*;
use crate::{AccessSource, BotPluginAccess, BotPluginAccessUpdate, BotRef, bot_plugin_access_path};

fn bot() -> BotRef {
    BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "alice".into(),
    }
}

fn request() -> InstallRequest {
    serde_json::from_value(json!({
        "source": "github:example/wonton-tools@v1.0.0",
        "digest": "sha256:abc",
        "grant": []
    }))
    .unwrap()
}

#[test]
fn runtimes_round_trip_with_their_wire_values() {
    for (runtime, wire) in [
        (PluginRuntime::Compose, "compose"),
        (PluginRuntime::RemoteMcp, "remote_mcp"),
        (PluginRuntime::Skills, "skills"),
        (PluginRuntime::Other, "other"),
    ] {
        assert_eq!(serde_json::to_value(runtime).unwrap(), json!(wire));
        assert_eq!(round_trip(&runtime), runtime);
        let installed = InstalledPlugin {
            runtime,
            ..installed()
        };
        let detail = PluginDetail {
            runtime,
            ..detail()
        };
        let preview = InstallPreview {
            runtime,
            ..preview()
        };
        assert_eq!(round_trip(&installed), installed);
        assert_eq!(round_trip(&detail), detail);
        assert_eq!(round_trip(&preview), preview);
    }
}

#[test]
fn scopes_round_trip_as_tagged_bot_selections() {
    for (scope, wire) in [
        (PluginScope::All, json!({"type": "all"})),
        (
            PluginScope::Bots { bots: vec![bot()] },
            json!({"type": "bots", "bots": [{"plugin_id": "ai.nous.hermes", "bot": "alice"}]}),
        ),
        (PluginScope::Other, json!({"type": "other"})),
    ] {
        assert_eq!(serde_json::to_value(&scope).unwrap(), wire);
        assert_eq!(round_trip(&scope), scope);
        let detail = PluginDetail {
            scope: Some(scope.clone()),
            ..detail()
        };
        let request = InstallRequest {
            scope: Some(scope),
            ..request()
        };
        assert_eq!(round_trip(&detail), detail);
        assert_eq!(round_trip(&request), request);
    }
    let preview = InstallPreview {
        scope_choices: vec![bot()],
        ..preview()
    };
    assert_eq!(round_trip(&preview), preview);
    assert_eq!(
        serde_json::to_value(preview).unwrap()["scope_choices"],
        json!([{"plugin_id": "ai.nous.hermes", "bot": "alice"}])
    );
}

#[test]
fn older_cores_and_apps_can_omit_plugin_forms_fields() {
    let mut installed = serde_json::to_value(installed()).unwrap();
    installed.as_object_mut().unwrap().remove("runtime");
    let installed: InstalledPlugin = serde_json::from_value(installed).unwrap();
    assert_eq!(installed.runtime, PluginRuntime::Compose);

    let mut detail = serde_json::to_value(detail()).unwrap();
    detail.as_object_mut().unwrap().remove("runtime");
    detail.as_object_mut().unwrap().remove("scope");
    let detail: PluginDetail = serde_json::from_value(detail).unwrap();
    assert_eq!(detail.runtime, PluginRuntime::Compose);
    assert_eq!(detail.scope, None);

    let mut preview = serde_json::to_value(preview()).unwrap();
    preview.as_object_mut().unwrap().remove("runtime");
    preview.as_object_mut().unwrap().remove("scope_choices");
    let preview: InstallPreview = serde_json::from_value(preview).unwrap();
    assert_eq!(preview.runtime, PluginRuntime::Compose);
    assert!(preview.scope_choices.is_empty());

    let request = request();
    assert_eq!(request.scope, None);
    assert!(
        serde_json::to_value(request)
            .unwrap()
            .get("scope")
            .is_none()
    );
}

#[test]
fn newer_plugin_forms_do_not_break_older_clients() {
    let mut installed = serde_json::to_value(installed()).unwrap();
    installed["runtime"] = json!("static_web");
    let installed: InstalledPlugin = serde_json::from_value(installed).unwrap();
    assert_eq!(installed.runtime, PluginRuntime::Other);

    let unknown_scope = json!({"type": "future_scope", "selection": ["future_bot"]});
    let mut detail = serde_json::to_value(detail()).unwrap();
    detail["runtime"] = json!("static_web");
    detail["scope"] = unknown_scope.clone();
    let detail: PluginDetail = serde_json::from_value(detail).unwrap();
    assert_eq!(detail.runtime, PluginRuntime::Other);
    assert_eq!(detail.scope, Some(PluginScope::Other));

    let mut preview = serde_json::to_value(preview()).unwrap();
    preview["runtime"] = json!("static_web");
    let preview: InstallPreview = serde_json::from_value(preview).unwrap();
    assert_eq!(preview.runtime, PluginRuntime::Other);

    let mut request = serde_json::to_value(request()).unwrap();
    request["scope"] = unknown_scope;
    let request: InstallRequest = serde_json::from_value(request).unwrap();
    assert_eq!(request.scope, Some(PluginScope::Other));
}

#[test]
fn bot_access_keeps_its_source_and_independent_switch() {
    for (source, wire) in [
        (AccessSource::Global, "global"),
        (AccessSource::Specified, "specified"),
        (AccessSource::Other, "other"),
    ] {
        for enabled in [false, true] {
            let access = BotPluginAccess {
                plugin_id: "app.tools".into(),
                source,
                enabled,
            };
            assert_eq!(round_trip(&access), access);
            assert_eq!(
                serde_json::to_value(access).unwrap(),
                json!({"plugin_id": "app.tools", "source": wire, "enabled": enabled})
            );
            let update = BotPluginAccessUpdate { enabled };
            assert_eq!(round_trip(&update), update);
            assert_eq!(
                serde_json::to_value(update).unwrap(),
                json!({"enabled": enabled})
            );
        }
    }
    let access: BotPluginAccess = serde_json::from_value(json!({
        "plugin_id": "app.tools", "source": "future_source", "enabled": true
    }))
    .unwrap();
    assert_eq!(access.source, AccessSource::Other);
}

#[test]
fn scope_and_bot_access_paths_keep_the_bot_identity() {
    assert_eq!(
        plugin_scope_path("app.tools"),
        "/_wonton/api/plugins/app.tools/scope"
    );
    assert_eq!(
        bot_plugin_access_path("ai.nous.hermes", "alice", "app.tools"),
        "/_wonton/api/bots/ai.nous.hermes/alice/plugin-access/app.tools"
    );
}
