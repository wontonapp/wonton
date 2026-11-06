//! 出网代理的设置页和网页局域网端口的开关（ADR-0023）。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::Method;
use wonton_client::{
    Action, CoreError, EgressCurrent, EgressFailure, EgressPathState, EgressProbe, EgressProblem,
    EgressProxyProvider, EgressProxySettings, EgressProxySource, EgressProxyUpdate, EgressRestart,
    LanWebuiOffer, PluginFailure,
};

use super::rig::{Canned, Rig, WAIT, core_error, preview};

const PATH: &str = "/_wonton/api/egress-proxy";

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn probed(problem: Option<EgressProblem>) -> Option<EgressProbe> {
    Some(EgressProbe {
        probed_at_ms: now_ms(),
        problem,
    })
}

fn unset() -> EgressProxySettings {
    EgressProxySettings {
        url: None,
        source: EgressProxySource::Unset,
        provider: None,
        plugin_path: None,
        core_path: None,
        host_cidrs: vec!["192.168.0.0/24".to_owned()],
        direct_cidrs: Vec::new(),
        notices: Vec::new(),
        restarts: vec![EgressRestart {
            id: "app.hermes".to_owned(),
            title: "Hermes".to_owned(),
        }],
    }
}

fn configured() -> EgressProxySettings {
    EgressProxySettings {
        url: Some("http://user:***@172.17.0.1:7899".to_owned()),
        source: EgressProxySource::Settings,
        provider: None,
        plugin_path: probed(None),
        core_path: probed(None),
        ..unset()
    }
}

fn available() -> EgressProxySettings {
    EgressProxySettings {
        provider: Some(EgressProxyProvider {
            id: "app.example.proxy".to_owned(),
            title: "Proxy".to_owned(),
        }),
        ..unset()
    }
}

fn plugin_proxy() -> EgressProxySettings {
    EgressProxySettings {
        source: EgressProxySource::Plugin,
        plugin_path: probed(None),
        core_path: probed(None),
        ..available()
    }
}

fn choose_plugin() -> Action {
    Action::SaveEgressProxy {
        address: String::new(),
        direct_cidrs: String::new(),
        use_plugin: true,
    }
}

fn ok(settings: &EgressProxySettings) -> Canned {
    Canned(200, serde_json::to_string(settings).unwrap())
}

async fn opened(rig: &Rig, settings: &EgressProxySettings) -> EgressCurrent {
    rig.connected().await;
    rig.core.answer(Method::GET, PATH, ok(settings));
    rig.app.dispatch(Action::RefreshEgressProxy).unwrap();
    rig.wait_for(|view| view.egress.current.is_some() && !view.egress.loading)
        .await
        .egress
        .current
        .unwrap()
}

async fn sent(rig: &Rig) -> EgressProxyUpdate {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(seen) = rig.core.seen(&Method::PUT, PATH).pop() {
            return serde_json::from_str(&seen.body).unwrap();
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_first_proxy_restarts_plugins_only_after_the_owner_confirms() {
    let rig = Rig::new().await;
    opened(&rig, &unset()).await;
    let save = || Action::SaveEgressProxy {
        address: " http://172.17.0.1:7899 ".to_owned(),
        direct_cidrs: "100.64.0.0/10, 10.1.0.0/16\n".to_owned(),
        use_plugin: false,
    };

    rig.app.dispatch(save()).unwrap();
    let confirmation = rig.app.plugins().egress.confirmation.unwrap();
    assert_eq!(confirmation.restarts[0].title, "Hermes");
    assert!(!confirmation.cancelling);
    rig.app.dispatch(Action::DismissEgressConfirmation).unwrap();
    assert!(rig.app.plugins().egress.confirmation.is_none());
    assert!(
        rig.core.seen(&Method::PUT, PATH).is_empty(),
        "没确认就什么都不改"
    );

    rig.core.answer(Method::PUT, PATH, ok(&configured()));
    rig.app.dispatch(save()).unwrap();
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    assert_eq!(
        sent(&rig).await,
        EgressProxyUpdate {
            url: Some("http://172.17.0.1:7899".to_owned()),
            direct_cidrs: vec!["100.64.0.0/10".to_owned(), "10.1.0.0/16".to_owned()],
            restarts: vec!["app.hermes".to_owned()],
            use_plugin: false,
        }
    );
    let view = rig
        .wait_for(|view| {
            !view.egress.saving && view.egress.current.as_ref().unwrap().settings.url.is_some()
        })
        .await;
    let current = view.egress.current.unwrap();
    assert_eq!(current.plugin_path, Some(EgressPathState::Reachable));
    assert_eq!(current.address, "http://user:***@172.17.0.1:7899");
}

#[tokio::test]
async fn changing_the_address_or_only_the_ranges_restarts_nothing() {
    let rig = Rig::new().await;
    let shown = opened(&rig, &configured()).await;
    rig.core.answer(Method::PUT, PATH, ok(&configured()));
    rig.app
        .dispatch(Action::SaveEgressProxy {
            address: shown.address.clone(),
            direct_cidrs: "100.64.0.0/10".to_owned(),
            use_plugin: false,
        })
        .unwrap();
    assert!(rig.app.plugins().egress.confirmation.is_none());
    let update = sent(&rig).await;
    assert_eq!(update.url, Some(shown.address), "打码地址原样带回表示不改");
    assert!(update.restarts.is_empty());
}

#[tokio::test]
async fn cancelling_the_proxy_is_confirmed_and_sends_no_address() {
    let rig = Rig::new().await;
    opened(&rig, &configured()).await;
    rig.core.answer(Method::PUT, PATH, ok(&unset()));
    rig.app
        .dispatch(Action::SaveEgressProxy {
            address: "  ".to_owned(),
            direct_cidrs: String::new(),
            use_plugin: false,
        })
        .unwrap();
    assert!(rig.app.plugins().egress.confirmation.unwrap().cancelling);
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    let update = sent(&rig).await;
    assert_eq!(update.url, None);
    assert_eq!(update.restarts, ["app.hermes"]);
}

/// 核心不存连不上的地址；要重启的插件可能变了，下一次确认用新的列表。
#[tokio::test]
async fn a_refused_save_says_why_and_the_next_confirmation_uses_the_new_list() {
    let rig = Rig::new().await;
    opened(&rig, &unset()).await;
    let mut newer = unset();
    newer.restarts.push(EgressRestart {
        id: "app.more".to_owned(),
        title: "More".to_owned(),
    });
    rig.core.answer(
        Method::PUT,
        PATH,
        core_error(422, "egress_proxy_unavailable", "5 秒没响应"),
    );
    rig.core.answer(Method::GET, PATH, ok(&newer));
    let save = || Action::SaveEgressProxy {
        address: "http://192.0.2.1:1".to_owned(),
        direct_cidrs: String::new(),
        use_plugin: false,
    };
    rig.app.dispatch(save()).unwrap();
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    let view = rig
        .wait_for(|view| {
            view.egress.save_problem.is_some()
                && view
                    .egress
                    .current
                    .as_ref()
                    .unwrap()
                    .settings
                    .restarts
                    .len()
                    == 2
        })
        .await;
    let problem = view.egress.save_problem.unwrap();
    assert_eq!(problem.reason, PluginFailure::EgressProxyUnavailable);
    assert_eq!(problem.message, "5 秒没响应");
    assert!(!view.egress.saving);

    rig.app.dispatch(save()).unwrap();
    assert_eq!(
        rig.app
            .plugins()
            .egress
            .confirmation
            .unwrap()
            .restarts
            .len(),
        2
    );
}

#[tokio::test]
async fn a_blocking_failure_and_an_uncertain_one_read_differently() {
    let rig = Rig::new().await;
    let problem = |reason, message: &str| {
        probed(Some(EgressProblem {
            reason,
            message: message.to_owned(),
        }))
    };
    let current = opened(
        &rig,
        &EgressProxySettings {
            plugin_path: problem(EgressFailure::ProxyUnreachable, "连接被拒绝"),
            core_path: problem(EgressFailure::UpstreamFailed, "代理连不上外面"),
            source: EgressProxySource::Env,
            ..configured()
        },
    )
    .await;
    assert_eq!(
        current.plugin_path,
        Some(EgressPathState::Failed {
            message: "连接被拒绝".to_owned()
        })
    );
    assert_eq!(
        current.core_path,
        Some(EgressPathState::Uncertain {
            message: "代理连不上外面".to_owned()
        })
    );
    assert!(current.address_locked, "地址来自环境变量，这里改不了");
}

/// 核心先给上次的结果、在后台再测（ADR-0023），新结果不用主人刷新就到。
#[tokio::test]
async fn an_old_probe_is_replaced_by_the_new_result_without_a_refresh() {
    let rig = Rig::new().await;
    let current = opened(
        &rig,
        &EgressProxySettings {
            plugin_path: None,
            core_path: None,
            ..configured()
        },
    )
    .await;
    assert_eq!(current.plugin_path, Some(EgressPathState::Probing));
    rig.core.answer(Method::GET, PATH, ok(&configured()));
    let view = rig
        .wait_for(|view| {
            view.egress.current.as_ref().unwrap().plugin_path == Some(EgressPathState::Reachable)
        })
        .await;
    assert!(view.egress.load_problem.is_none());
}

#[tokio::test]
async fn no_proxy_has_no_paths_and_a_failed_read_says_why() {
    let rig = Rig::new().await;
    let current = opened(&rig, &unset()).await;
    assert_eq!(current.plugin_path, None);
    assert_eq!(current.address, "");

    rig.core
        .answer(Method::GET, PATH, core_error(500, "docker", "Docker 出错"));
    rig.app.dispatch(Action::RefreshEgressProxy).unwrap();
    let view = rig
        .wait_for(|view| view.egress.load_problem.is_some())
        .await;
    assert_eq!(
        view.egress.load_problem.unwrap().reason,
        PluginFailure::Upstream
    );
    assert!(view.egress.current.is_some(), "显示上次取到的");
}

#[tokio::test]
async fn the_choice_on_the_confirmation_page_reaches_the_core() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut shown = preview("app.photos");
    shown.lan_webui = Some(LanWebuiOffer {
        ports: Vec::new(),
        risk: "风险".to_owned(),
        enabled: true,
        client: None,
        client_problem: None,
    });
    rig.app
        .dispatch(Action::InstallPlugin { preview: shown })
        .unwrap();
    let path = "/_wonton/api/plugins";
    let deadline = Instant::now() + WAIT;
    while rig.core.seen(&Method::POST, path).is_empty() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let body: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, path)[0].body).unwrap();
    assert_eq!(body["lan_webui"], true);
}

#[tokio::test]
async fn a_refused_toggle_reports_the_reason_of_the_core() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = "/_wonton/api/plugins/app.photos/lan-webui";
    rig.core.answer(
        Method::PUT,
        path,
        core_error(409, "lan_unavailable", "局域网地址选不出来"),
    );
    let result = rig.app.set_lan_webui("app.photos".to_owned(), true).await;
    assert!(
        matches!(result, Err(CoreError::NotReady { reason }) if reason == "局域网地址选不出来")
    );
    let seen = &rig.core.seen(&Method::PUT, path)[0];
    assert_eq!(seen.body, r#"{"enabled":true}"#);
}

#[tokio::test]
async fn selecting_an_installed_proxy_requires_confirmation_and_can_be_dismissed() {
    let rig = Rig::new().await;
    let current = opened(&rig, &available()).await;
    assert!(!current.use_plugin);
    assert!(current.settings.provider.is_some());
    rig.app.dispatch(choose_plugin()).unwrap();
    let confirmation = rig.app.plugins().egress.confirmation.unwrap();
    assert!(!confirmation.cancelling);
    assert_eq!(confirmation.restarts, available().restarts);
    rig.app.dispatch(Action::DismissEgressConfirmation).unwrap();
    assert!(rig.app.plugins().egress.confirmation.is_none());
    assert!(rig.core.seen(&Method::PUT, PATH).is_empty());
    assert!(!rig.app.plugins().egress.current.unwrap().use_plugin);

    rig.core.answer(Method::PUT, PATH, ok(&plugin_proxy()));
    rig.app.dispatch(choose_plugin()).unwrap();
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    let update = sent(&rig).await;
    assert!(update.use_plugin);
    assert_eq!(update.url, None);
    assert_eq!(update.restarts, ["app.hermes"]);
    let view = rig
        .wait_for(|view| !view.egress.saving && view.egress.current.as_ref().unwrap().use_plugin)
        .await;
    let current = view.egress.current.unwrap();
    assert!(current.address.is_empty());
    assert_eq!(current.plugin_path, Some(EgressPathState::Reachable));
}

#[tokio::test]
async fn switching_manual_and_plugin_proxies_never_requests_plugin_restarts() {
    let rig = Rig::new().await;
    let manual = EgressProxySettings {
        provider: available().provider,
        ..configured()
    };
    opened(&rig, &manual).await;
    rig.core.answer(Method::PUT, PATH, ok(&plugin_proxy()));
    rig.app.dispatch(choose_plugin()).unwrap();
    assert!(rig.app.plugins().egress.confirmation.is_none());
    let update = sent(&rig).await;
    assert!(update.use_plugin);
    assert!(update.restarts.is_empty());
    rig.wait_for(|view| !view.egress.saving && view.egress.current.as_ref().unwrap().use_plugin)
        .await;

    rig.core.answer(Method::PUT, PATH, ok(&manual));
    rig.app
        .dispatch(Action::SaveEgressProxy {
            address: manual.url.clone().unwrap(),
            direct_cidrs: String::new(),
            use_plugin: false,
        })
        .unwrap();
    rig.wait_for(|view| !view.egress.saving && !view.egress.current.as_ref().unwrap().use_plugin)
        .await;
    assert!(rig.app.plugins().egress.confirmation.is_none());
    let updates = rig.core.seen(&Method::PUT, PATH);
    assert_eq!(updates.len(), 2);
    let update: EgressProxyUpdate = serde_json::from_str(&updates[1].body).unwrap();
    assert!(!update.use_plugin);
    assert_eq!(update.url, manual.url);
    assert!(update.restarts.is_empty());
}

#[tokio::test]
async fn cancelling_the_plugin_proxy_requires_restart_confirmation() {
    let rig = Rig::new().await;
    opened(&rig, &plugin_proxy()).await;
    rig.core.answer(Method::PUT, PATH, ok(&available()));
    rig.app
        .dispatch(Action::SaveEgressProxy {
            address: String::new(),
            direct_cidrs: String::new(),
            use_plugin: false,
        })
        .unwrap();
    assert!(rig.app.plugins().egress.confirmation.unwrap().cancelling);
    assert!(rig.core.seen(&Method::PUT, PATH).is_empty());
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    let update = sent(&rig).await;
    assert!(!update.use_plugin);
    assert_eq!(update.url, None);
    assert_eq!(update.restarts, ["app.hermes"]);
    let view = rig
        .wait_for(|view| !view.egress.saving && !view.egress.current.as_ref().unwrap().use_plugin)
        .await;
    assert_eq!(view.egress.current.unwrap().plugin_path, None);
}

#[tokio::test]
async fn invalid_plugin_choices_never_discard_an_address_or_send_a_request() {
    let rig = Rig::new().await;
    opened(&rig, &unset()).await;
    assert!(matches!(
        rig.app.dispatch(choose_plugin()),
        Err(CoreError::NotReady { .. })
    ));
    assert!(rig.app.plugins().egress.confirmation.is_none());
    assert!(rig.core.seen(&Method::PUT, PATH).is_empty());

    rig.core.answer(Method::GET, PATH, ok(&available()));
    rig.app.dispatch(Action::RefreshEgressProxy).unwrap();
    rig.wait_for(|view| {
        view.egress
            .current
            .as_ref()
            .unwrap()
            .settings
            .provider
            .is_some()
    })
    .await;
    assert!(matches!(
        rig.app.dispatch(Action::SaveEgressProxy {
            address: "http://192.0.2.10:7890".to_owned(),
            direct_cidrs: String::new(),
            use_plugin: true,
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(rig.app.plugins().egress.confirmation.is_none());
    assert!(rig.core.seen(&Method::PUT, PATH).is_empty());
}

#[tokio::test]
async fn environment_proxy_selection_is_locked_but_ranges_can_change() {
    let rig = Rig::new().await;
    let settings = EgressProxySettings {
        source: EgressProxySource::Env,
        provider: available().provider,
        ..configured()
    };
    let current = opened(&rig, &settings).await;
    assert!(current.address_locked);
    assert!(matches!(
        rig.app.dispatch(choose_plugin()),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(rig.core.seen(&Method::PUT, PATH).is_empty());
    rig.core.answer(Method::PUT, PATH, ok(&settings));
    rig.app
        .dispatch(Action::SaveEgressProxy {
            address: current.address,
            direct_cidrs: "100.64.0.0/10".to_owned(),
            use_plugin: false,
        })
        .unwrap();
    let update = sent(&rig).await;
    assert_eq!(update.direct_cidrs, ["100.64.0.0/10"]);
    assert!(!update.use_plugin);
}

#[tokio::test]
async fn plugin_paths_follow_unsettled_probes_and_keep_failures_visible() {
    let rig = Rig::new().await;
    let settings = EgressProxySettings {
        plugin_path: None,
        core_path: None,
        ..plugin_proxy()
    };
    let current = opened(&rig, &settings).await;
    assert!(current.use_plugin);
    assert_eq!(current.plugin_path, Some(EgressPathState::Probing));
    assert_eq!(current.core_path, Some(EgressPathState::Probing));
    let failed = EgressProxySettings {
        plugin_path: probed(Some(EgressProblem {
            reason: EgressFailure::ProxyUnreachable,
            message: "代理插件未运行".to_owned(),
        })),
        core_path: probed(Some(EgressProblem {
            reason: EgressFailure::UpstreamFailed,
            message: "上游不可用".to_owned(),
        })),
        ..plugin_proxy()
    };
    rig.core.answer(Method::GET, PATH, ok(&failed));
    let view = rig
        .wait_for(|view| {
            matches!(
                view.egress.current.as_ref().unwrap().plugin_path,
                Some(EgressPathState::Failed { .. })
            )
        })
        .await;
    let current = view.egress.current.unwrap();
    assert!(current.use_plugin);
    assert!(matches!(
        current.core_path,
        Some(EgressPathState::Uncertain { .. })
    ));
    assert_eq!(current.settings.url, None);
}

#[tokio::test]
async fn saving_a_plugin_proxy_follows_probes_until_the_new_result_arrives() {
    let rig = Rig::new().await;
    opened(&rig, &available()).await;
    rig.core.answer(
        Method::PUT,
        PATH,
        ok(&EgressProxySettings {
            plugin_path: None,
            core_path: None,
            ..plugin_proxy()
        }),
    );
    rig.core
        .answer_always(Method::GET, PATH, ok(&plugin_proxy()));
    rig.app.dispatch(choose_plugin()).unwrap();
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    rig.wait_for(|view| {
        let current = view.egress.current.as_ref().unwrap();
        !view.egress.saving
            && current.use_plugin
            && current.plugin_path == Some(EgressPathState::Reachable)
    })
    .await;
    assert!(rig.core.seen(&Method::GET, PATH).len() >= 2);
}

#[tokio::test]
async fn a_failed_plugin_save_preserves_the_current_selection_and_reports_the_problem() {
    let rig = Rig::new().await;
    opened(&rig, &available()).await;
    rig.core.answer(
        Method::PUT,
        PATH,
        core_error(404, "not_installed", "插件已卸载"),
    );
    rig.core.answer(Method::GET, PATH, ok(&unset()));
    rig.app.dispatch(choose_plugin()).unwrap();
    rig.app.dispatch(Action::ConfirmEgressProxy).unwrap();
    let view = rig
        .wait_for(|view| {
            view.egress.save_problem.is_some()
                && view
                    .egress
                    .current
                    .as_ref()
                    .unwrap()
                    .settings
                    .provider
                    .is_none()
        })
        .await;
    assert!(!view.egress.saving);
    assert!(!view.egress.current.unwrap().use_plugin);
    assert_eq!(
        view.egress.save_problem.unwrap().reason,
        PluginFailure::NotFound
    );
}

#[tokio::test]
async fn an_old_plugin_probe_is_refreshed_even_without_a_proxy_url() {
    let rig = Rig::new().await;
    let stale = EgressProxySettings {
        plugin_path: Some(EgressProbe {
            probed_at_ms: 0,
            problem: Some(EgressProblem {
                reason: EgressFailure::ProxyUnreachable,
                message: "代理插件未运行".to_owned(),
            }),
        }),
        ..plugin_proxy()
    };
    let current = opened(&rig, &stale).await;
    assert!(matches!(
        current.plugin_path,
        Some(EgressPathState::Failed { .. })
    ));
    rig.core
        .answer_always(Method::GET, PATH, ok(&plugin_proxy()));
    rig.wait_for(|view| {
        view.egress.current.as_ref().unwrap().plugin_path == Some(EgressPathState::Reachable)
    })
    .await;
}
