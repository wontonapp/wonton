//! 镜像加速设置经真实隧道到假核心的行为测试。

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, CoreError, PluginFailure, RegistryMirrorSettings, RegistryMirrorTest,
    RegistryMirrorTestRequest, RegistryMirrorTests, RegistryMirrorUpdate, RegistryProbe,
};

use super::rig::{Canned, FakeHome, Rig, WAIT, core_error};

const PATH: &str = "/_wonton/api/registry-mirror";
const TEST: &str = "/_wonton/api/registry-mirror/test";

fn settings(mirror: Option<&str>) -> RegistryMirrorSettings {
    RegistryMirrorSettings {
        mirror: mirror.map(str::to_owned),
        choices: vec![
            "first.example".into(),
            "second.example".into(),
            "first.example".into(),
        ],
        choices_problem: None,
        docker_hub: None,
        recommended: false,
    }
}

fn ok(value: &impl serde::Serialize) -> Canned {
    Canned(200, serde_json::to_string(value).unwrap())
}

fn tests(mirror: &str) -> RegistryMirrorTests {
    RegistryMirrorTests {
        docker_hub: None,
        mirrors: vec![RegistryMirrorTest {
            mirror: mirror.into(),
            probe: RegistryProbe {
                probed_at_ms: 1,
                latency_ms: Some(12),
                problem: None,
            },
        }],
    }
}

async fn opened(rig: &Rig, current: &RegistryMirrorSettings) {
    rig.connected().await;
    rig.core.answer(Method::GET, PATH, ok(current));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshRegistryMirror,
        })
        .unwrap();
    let vm = rig
        .wait_for(|view| view.registry_mirror.settings.is_some() && !view.registry_mirror.loading)
        .await
        .registry_mirror;
    assert_eq!(vm.settings.as_ref(), Some(current));
}

async fn seen(rig: &Rig, method: Method, path: &str) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(request) = rig.core.seen(&method, path).last() {
            return request.body.clone();
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn save(rig: &Rig, mirror: Option<&str>) {
    rig.core.answer(Method::PUT, PATH, ok(&settings(mirror)));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::SaveRegistryMirror {
                mirror: mirror.unwrap_or_default().into(),
            },
        })
        .unwrap();
    let vm = rig
        .wait_for(|view| !view.registry_mirror.saving)
        .await
        .registry_mirror;
    assert!(vm.save_problem.is_none());
    assert_eq!(vm.settings.unwrap().mirror.as_deref(), mirror);
}

#[tokio::test]
async fn registry_refresh_save_disable_and_test_choices_include_custom_current() {
    let rig = Rig::new().await;
    opened(&rig, &settings(None)).await;
    save(&rig, Some("custom.example:5000")).await;
    let update: RegistryMirrorUpdate =
        serde_json::from_str(&seen(&rig, Method::PUT, PATH).await).unwrap();
    assert_eq!(update.mirror.as_deref(), Some("custom.example:5000"));
    rig.core
        .answer(Method::POST, TEST, ok(&tests("custom.example:5000")));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::TestRegistryMirrors,
        })
        .unwrap();
    let vm = rig
        .wait_for(|view| !view.registry_mirror.testing)
        .await
        .registry_mirror;
    let request: RegistryMirrorTestRequest =
        serde_json::from_str(&seen(&rig, Method::POST, TEST).await).unwrap();
    assert_eq!(
        request.mirrors,
        ["first.example", "second.example", "custom.example:5000"]
    );
    assert_eq!(vm.tests, Some(tests("custom.example:5000")));
    save(&rig, None).await;
    let update: RegistryMirrorUpdate =
        serde_json::from_str(&rig.core.seen(&Method::PUT, PATH).last().unwrap().body).unwrap();
    assert_eq!(update.mirror, None);
}

#[tokio::test]
async fn registry_failures_remain_visible_and_do_not_discard_settings() {
    let rig = Rig::new().await;
    opened(&rig, &settings(Some("custom.example"))).await;
    rig.core
        .answer(Method::GET, PATH, core_error(500, "docker", "read detail"));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshRegistryMirror,
        })
        .unwrap();
    let vm = rig
        .wait_for(|v| v.registry_mirror.load_problem.is_some())
        .await
        .registry_mirror;
    assert_eq!(vm.load_problem.unwrap().reason, PluginFailure::Upstream);
    rig.core.answer(
        Method::PUT,
        PATH,
        core_error(400, "invalid_input", "invalid host"),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::SaveRegistryMirror {
                mirror: "https://bad.example".into(),
            },
        })
        .unwrap();
    let vm = rig
        .wait_for(|v| v.registry_mirror.save_problem.is_some())
        .await
        .registry_mirror;
    assert_eq!(vm.save_problem.unwrap().reason, PluginFailure::Invalid);
    rig.core.answer(
        Method::POST,
        TEST,
        core_error(403, "cross_site", "owner required"),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::TestRegistryMirrors,
        })
        .unwrap();
    let vm = rig
        .wait_for(|v| v.registry_mirror.test_problem.is_some())
        .await
        .registry_mirror;
    assert_eq!(vm.test_problem.unwrap().reason, PluginFailure::Unauthorized);
    assert!(vm.load_problem.is_some());
    assert!(vm.save_problem.is_some());
    assert_eq!(vm.settings, Some(settings(Some("custom.example"))));
}

#[tokio::test]
async fn registry_old_refresh_and_test_cannot_undo_a_save() {
    let rig = Rig::new().await;
    opened(&rig, &settings(Some("old.example"))).await;
    rig.core.hold(Method::GET, PATH);
    rig.core
        .answer(Method::GET, PATH, ok(&settings(Some("old.example"))));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshRegistryMirror,
        })
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while rig.core.seen(&Method::GET, PATH).len() < 2 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    rig.core.hold(Method::POST, TEST);
    rig.core
        .answer(Method::POST, TEST, ok(&tests("old.example")));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::TestRegistryMirrors,
        })
        .unwrap();
    seen(&rig, Method::POST, TEST).await;
    save(&rig, Some("new.example")).await;
    rig.core.release(Method::GET, PATH);
    rig.core.release(Method::POST, TEST);
    let vm = rig
        .wait_for(|v| !v.registry_mirror.testing)
        .await
        .registry_mirror;
    assert_eq!(vm.settings, Some(settings(Some("new.example"))));
    // 旧 GET 也可能比测速响应晚到；每次通知都检查地址，不能只检查最终一次。
    let subscription = rig.app.subscribe_plugins();
    let deadline = tokio::time::sleep(Duration::from_millis(200));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            view = subscription.next() => {
                assert_eq!(view.unwrap().registry_mirror.settings, Some(settings(Some("new.example"))));
            }
        }
    }
}

#[tokio::test]
async fn registry_concurrent_saves_are_rejected_until_the_first_settles() {
    let rig = Rig::new().await;
    opened(&rig, &settings(None)).await;
    rig.core.hold(Method::PUT, PATH);
    rig.core
        .answer(Method::PUT, PATH, ok(&settings(Some("first.example"))));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::SaveRegistryMirror {
                mirror: "first.example".into(),
            },
        })
        .unwrap();
    seen(&rig, Method::PUT, PATH).await;
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::SaveRegistryMirror {
                mirror: "second.example".into(),
            },
        }),
        Err(CoreError::NotReady { .. })
    ));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshRegistryMirror,
        })
        .unwrap();
    rig.core.release(Method::PUT, PATH);
    let vm = rig
        .wait_for(|v| !v.registry_mirror.saving)
        .await
        .registry_mirror;
    assert_eq!(vm.settings, Some(settings(Some("first.example"))));
}

#[tokio::test]
async fn registry_previous_server_responses_do_not_populate_the_new_server() {
    let rig = Rig::new().await;
    opened(&rig, &settings(Some("old.example"))).await;
    rig.core.hold(Method::PUT, PATH);
    rig.core
        .answer(Method::PUT, PATH, ok(&settings(Some("late.example"))));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::SaveRegistryMirror {
                mirror: "late.example".into(),
            },
        })
        .unwrap();
    seen(&rig, Method::PUT, PATH).await;
    let other = FakeHome::start(vec![rig.device()]).await;
    rig.connect_to(other.ticket());
    rig.wait_for(|v| v.registry_mirror.settings.is_none() && !v.registry_mirror.saving)
        .await;
    let deadline = Instant::now() + WAIT;
    while rig.app.home().status != wonton_client::HomeStatus::Connected {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    other
        .core
        .answer(Method::GET, PATH, ok(&settings(Some("new-server.example"))));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshRegistryMirror,
        })
        .unwrap();
    rig.wait_for(|v| v.registry_mirror.settings.is_some()).await;
    rig.core.release(Method::PUT, PATH);
    let subscription = rig.app.subscribe_plugins();
    let deadline = tokio::time::sleep(Duration::from_millis(200));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            view = subscription.next() => assert_eq!(
                view.unwrap().registry_mirror.settings,
                Some(settings(Some("new-server.example"))),
            ),
        }
    }
}

#[tokio::test]
async fn registry_save_without_a_connection_reports_transport_failure() {
    let rig = Rig::new().await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::SaveRegistryMirror {
                mirror: "custom.example".into(),
            },
        })
        .unwrap();
    let vm = rig
        .wait_for(|v| v.registry_mirror.save_problem.is_some())
        .await
        .registry_mirror;
    assert!(!vm.saving);
    assert_eq!(
        vm.save_problem.unwrap().reason,
        PluginFailure::TransportUnavailable
    );
    assert!(vm.settings.is_none());
}

#[tokio::test]
async fn registry_test_refreshes_recommendation_without_replacing_the_selected_mirror() {
    let rig = Rig::new().await;
    opened(&rig, &settings(Some("custom.example"))).await;
    let mut result = tests("custom.example");
    result.docker_hub = Some(RegistryProbe {
        probed_at_ms: 2,
        latency_ms: None,
        problem: Some(wonton_client::RegistryProblem {
            reason: wonton_client::RegistryFailure::Unreachable,
            message: "unreachable".into(),
        }),
    });
    rig.core.answer(Method::POST, TEST, ok(&result));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::TestRegistryMirrors,
        })
        .unwrap();
    let view = rig.wait_for(|view| !view.registry_mirror.testing).await;
    let current = view.registry_mirror.settings.unwrap();
    assert_eq!(current.mirror.as_deref(), Some("custom.example"));
    assert!(current.recommended);
    result.docker_hub = None;
    rig.core.answer(Method::POST, TEST, ok(&result));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::TestRegistryMirrors,
        })
        .unwrap();
    let view = rig.wait_for(|view| !view.registry_mirror.testing).await;
    let current = view.registry_mirror.settings.unwrap();
    assert!(!current.recommended);
    assert_eq!(current.mirror.as_deref(), Some("custom.example"));
}

/// 索引的地址列表读不懂：设置照常显示，问题作为加载问题露出来，重试能再拉一次。
#[tokio::test]
async fn unreadable_index_choices_show_a_problem_and_keep_the_settings() {
    let rig = Rig::new().await;
    let mut unreadable = settings(Some("custom.example"));
    unreadable.choices.clear();
    unreadable.choices_problem = Some("索引的 registry_mirrors 读不懂：invalid type".into());
    opened(&rig, &unreadable).await;
    let vm = rig
        .wait_for(|view| view.registry_mirror.load_problem.is_some())
        .await
        .registry_mirror;
    let problem = vm.load_problem.unwrap();
    assert_eq!(problem.reason, PluginFailure::Upstream);
    assert!(problem.message.contains("registry_mirrors"), "{problem:?}");
    assert_eq!(vm.settings, Some(unreadable));
}
