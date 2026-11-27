//! 一个 Bot 的模型页（ADR-0129）的行为测试：动作各打各的接口，登录自己轮询到有结果，失败按类别落进 ViewModel。
//! 对面是本机假扮的核心。

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_client::{
    Action, AgentModelsAction, AgentModelsReason, AgentModelsState, AgentModelsViewModel,
    KeyCheckResult, LoginState, ModelListState, ModelLoginBrowser, ModelTestResult, ModelTestState,
    SecretString,
};
use wonton_protocol::{
    BotRef, MODEL_LOGIN_REDIRECT_URI, agent_model_current_path, agent_model_list_path,
    agent_model_login_finish_path, agent_model_login_path, agent_model_login_start_path,
    agent_model_provider_path, agent_model_test_path, agent_models_path,
};

use crate::plugins::rig::{Canned, Rig, WAIT, core_error};

const PLUGIN: &str = "ai.nous.hermes";
const BOT: &str = "default";
/// 插件拼的会话号带 `:`：路径里要编成一段。
const SESSION: &str = "nous:abc/1";

fn bot() -> BotRef {
    BotRef {
        plugin_id: PLUGIN.into(),
        bot: BOT.into(),
    }
}

fn ok(value: Value) -> Canned {
    Canned(StatusCode::OK.as_u16(), value.to_string())
}

fn provider(id: &str, name: &str, recommended: bool, auth: Value, connected: bool) -> Value {
    json!({
        "id": id, "name": name, "recommended": recommended, "auth": auth,
        "connected": connected, "account": null,
    })
}

fn key_auth() -> Value {
    json!([{"type": "api_key", "key_url": "https://openrouter.example/keys", "key_prefix": "sk-or-"}])
}

fn catalog(connected: bool) -> Value {
    json!({
        "providers": [
            provider("openrouter", "OpenRouter", true, key_auth(), connected),
            provider("nous", "Nous Portal", true, json!([{"type": "device_login"}]), false),
            provider("deepseek", "DeepSeek", false, key_auth(), false),
            provider("xai", "xAI", false, key_auth(), false),
            provider("future", "Future", false, json!([{"type": "quantum_login"}]), false),
        ],
        "current": null,
    })
}

fn models() -> Canned {
    ok(json!({"models": [
        {"id": "anthropic/claude-sonnet", "name": null, "recommended": true},
        {"id": "openai/gpt", "name": "GPT", "recommended": false},
    ]}))
}

fn act(rig: &Rig, action: AgentModelsAction) {
    rig.app
        .dispatch(Action::AgentModels { action })
        .expect("动作被拒");
}

async fn wait(
    rig: &Rig,
    mut done: impl FnMut(&AgentModelsViewModel) -> bool,
) -> AgentModelsViewModel {
    let updates = rig.app.subscribe_agent_models();
    let waiting = async {
        loop {
            let view = rig.app.agent_models();
            if done(&view) {
                return view;
            }
            updates.next().await.expect("订阅提前结束");
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", rig.app.agent_models()))
}

/// 连上、打开默认 Bot 的模型页，等目录到。
async fn opened(connected: bool) -> Rig {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer_always(
        Method::GET,
        &agent_models_path(PLUGIN, BOT),
        ok(catalog(connected)),
    );
    act(&rig, AgentModelsAction::OpenAgentModels { bot: bot() });
    wait(&rig, |view| view.state == AgentModelsState::Ready).await;
    rig
}

fn ids(rows: &[wonton_client::ModelProviderRow]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

#[tokio::test]
async fn providers_split_into_recommended_and_a_searchable_rest() {
    let rig = opened(false).await;
    let view = rig.app.agent_models();
    assert_eq!(ids(&view.recommended), ["openrouter", "nous"]);
    // App 不认识的接法那家不列。
    assert_eq!(ids(&view.more), ["deepseek", "xai"]);

    act(
        &rig,
        AgentModelsAction::SearchModelProviders {
            text: "DEEP".into(),
        },
    );
    let view = wait(&rig, |view| view.more.len() == 1).await;
    assert_eq!(ids(&view.more), ["deepseek"]);
    assert_eq!(view.recommended.len(), 2);
}

/// 存 key：请求体带 key，服务商认了就显示“已接好”并取出模型。
#[tokio::test]
async fn a_saved_key_connects_the_provider_and_lists_its_models() {
    let rig = opened(false).await;
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "openrouter".into(),
        },
    );
    let view = wait(&rig, |view| view.provider.is_some()).await;
    let page = view.provider.unwrap();
    assert_eq!(page.models, ModelListState::NotConnected);
    assert_eq!(page.key.unwrap().key_prefix.as_deref(), Some("sk-or-"));

    let path = agent_model_provider_path(PLUGIN, BOT, "openrouter");
    rig.core
        .answer(Method::PUT, &path, ok(json!({"check": "accepted"})));
    rig.core.answer_always(
        Method::GET,
        &agent_models_path(PLUGIN, BOT),
        ok(catalog(true)),
    );
    rig.core.answer(
        Method::GET,
        &agent_model_list_path(PLUGIN, BOT, "openrouter"),
        models(),
    );
    act(
        &rig,
        AgentModelsAction::SaveModelKey {
            key: SecretString::new("sk-or-secret"),
            base_url: None,
        },
    );
    let view = wait(&rig, |view| {
        view.provider.as_ref().is_some_and(|page| {
            page.row.connected && matches!(page.models, ModelListState::Ready { .. })
        })
    })
    .await;
    let page = view.provider.unwrap();
    assert!(!page.busy);
    assert_eq!(page.key_check.unwrap().check, KeyCheckResult::Accepted);
    let body: Value = serde_json::from_str(&rig.core.seen(&Method::PUT, &path)[0].body).unwrap();
    assert_eq!(body, json!({"key": "sk-or-secret"}));
}

/// 填错的 key：页面照旧，原因带上是哪一项，壳标到输入框旁。
#[tokio::test]
async fn a_rejected_input_names_the_field_and_keeps_the_page() {
    let rig = opened(false).await;
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "deepseek".into(),
        },
    );
    let body = json!({"error": {"kind": "invalid_input", "message": "不像 key", "field": "key"}});
    rig.core.answer(
        Method::PUT,
        &agent_model_provider_path(PLUGIN, BOT, "deepseek"),
        Canned(400, body.to_string()),
    );
    act(
        &rig,
        AgentModelsAction::SaveModelKey {
            key: SecretString::new("x"),
            base_url: None,
        },
    );
    let view = wait(&rig, |view| view.action_problem.is_some()).await;
    let problem = view.action_problem.unwrap();
    assert_eq!(problem.reason, AgentModelsReason::InvalidInput);
    assert_eq!(problem.field.as_deref(), Some("key"));
    assert_eq!(view.state, AgentModelsState::Ready);
    assert!(!view.provider.unwrap().busy);
}

/// 设备码登录：打开插件给的网址、显示码，自己轮询，批准了就收起登录、取出模型。
#[tokio::test]
async fn a_device_login_polls_until_approved() {
    let rig = opened(false).await;
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "nous".into(),
        },
    );
    let login = json!({
        "session": SESSION, "url": "https://portal.example/device?code=WXYZ",
        "code": "WXYZ", "expires_in": 600, "interval": 1,
    });
    let start = agent_model_login_start_path(PLUGIN, BOT, "nous");
    rig.core.answer(Method::POST, &start, ok(login));
    let poll = agent_model_login_path(PLUGIN, BOT, SESSION);
    rig.core
        .answer(Method::GET, &poll, ok(json!({"state": "pending"})));
    rig.core
        .answer(Method::GET, &poll, ok(json!({"state": "approved"})));
    let mut connected = catalog(false);
    connected["providers"][1]["connected"] = true.into();
    rig.core
        .answer_always(Method::GET, &agent_models_path(PLUGIN, BOT), ok(connected));
    rig.core.answer(
        Method::GET,
        &agent_model_list_path(PLUGIN, BOT, "nous"),
        models(),
    );
    act(&rig, AgentModelsAction::StartModelLogin);

    let view = wait(&rig, |view| {
        view.provider
            .as_ref()
            .is_some_and(|page| page.login.is_some())
    })
    .await;
    let progress = view.provider.unwrap().login.unwrap();
    assert_eq!(progress.browser, ModelLoginBrowser::Page);
    assert_eq!(progress.code.as_deref(), Some("WXYZ"));
    let start_body: Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, &start)[0].body).unwrap();
    assert_eq!(start_body, json!({}));

    let view = wait(&rig, |view| {
        view.provider.as_ref().is_some_and(|page| {
            page.login.is_none() && matches!(page.models, ModelListState::Ready { .. })
        })
    })
    .await;
    let page = view.provider.unwrap();
    assert!(page.row.connected);
    assert_eq!(page.login_outcome, None);
    assert_eq!(rig.core.seen(&Method::GET, &poll).len(), 2);
}

/// 浏览器授权：开始时带上 App 的回跳地址，壳交回回跳网址后由插件换凭据；对方拒绝时如实显示。
#[tokio::test]
async fn a_browser_login_finishes_with_the_callback_url() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut browser = catalog(false);
    browser["providers"][0]["auth"] = json!([{"type": "browser_login"}]);
    rig.core
        .answer_always(Method::GET, &agent_models_path(PLUGIN, BOT), ok(browser));
    act(&rig, AgentModelsAction::OpenAgentModels { bot: bot() });
    wait(&rig, |view| view.state == AgentModelsState::Ready).await;
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "openrouter".into(),
        },
    );
    let start = agent_model_login_start_path(PLUGIN, BOT, "openrouter");
    rig.core.answer(
        Method::POST,
        &start,
        ok(json!({
            "session": SESSION, "url": "https://openrouter.example/auth",
            "expires_in": 600, "interval": 5,
        })),
    );
    act(&rig, AgentModelsAction::StartModelLogin);
    let view = wait(&rig, |view| {
        view.provider
            .as_ref()
            .is_some_and(|page| page.login.is_some())
    })
    .await;
    assert_eq!(
        view.provider.unwrap().login.unwrap().browser,
        ModelLoginBrowser::Redirect
    );
    let start_body: Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, &start)[0].body).unwrap();
    assert_eq!(start_body["redirect_uri"], MODEL_LOGIN_REDIRECT_URI);

    let finish = agent_model_login_finish_path(PLUGIN, BOT, SESSION);
    rig.core.answer(
        Method::POST,
        &finish,
        ok(json!({"state": "denied", "message": "你拒绝了授权"})),
    );
    let callback = format!("{MODEL_LOGIN_REDIRECT_URI}?code=abc");
    act(
        &rig,
        AgentModelsAction::FinishModelLogin {
            callback_url: callback.clone(),
        },
    );
    let view = wait(&rig, |view| {
        view.provider
            .as_ref()
            .is_some_and(|page| page.login_outcome.is_some())
    })
    .await;
    let page = view.provider.unwrap();
    assert!(page.login.is_none());
    assert_eq!(page.login_outcome.unwrap().state, LoginState::Denied);
    let body: Value = serde_json::from_str(&rig.core.seen(&Method::POST, &finish)[0].body).unwrap();
    assert_eq!(body["callback_url"], callback);
}

/// 关掉这家就取消登录：页面马上收起，插件那边的会话也取消掉。
#[tokio::test]
async fn closing_the_provider_cancels_its_login() {
    let rig = opened(false).await;
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "nous".into(),
        },
    );
    rig.core.answer(
        Method::POST,
        &agent_model_login_start_path(PLUGIN, BOT, "nous"),
        ok(json!({
            "session": SESSION, "url": "https://portal.example/device",
            "code": "WXYZ", "expires_in": 600, "interval": 30,
        })),
    );
    act(&rig, AgentModelsAction::StartModelLogin);
    wait(&rig, |view| {
        view.provider
            .as_ref()
            .is_some_and(|page| page.login.is_some())
    })
    .await;
    let cancel = agent_model_login_path(PLUGIN, BOT, SESSION);
    rig.core
        .answer(Method::DELETE, &cancel, Canned(204, String::new()));
    act(&rig, AgentModelsAction::CloseModelProvider);
    assert!(rig.app.agent_models().provider.is_none());
    let deadline = tokio::time::Instant::now() + WAIT;
    while rig.core.seen(&Method::DELETE, &cancel).is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "没有取消登录");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// 插件提示贵的模型先不存，确认后带上 `confirmed` 再存；换了模型就清掉上一次的测试。
#[tokio::test]
async fn an_expensive_model_waits_for_confirmation() {
    let rig = opened(true).await;
    rig.core.answer(
        Method::GET,
        &agent_model_list_path(PLUGIN, BOT, "openrouter"),
        models(),
    );
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "openrouter".into(),
        },
    );
    wait(&rig, |view| {
        view.provider
            .as_ref()
            .is_some_and(|page| matches!(page.models, ModelListState::Ready { .. }))
    })
    .await;
    rig.core.answer(
        Method::POST,
        &agent_model_test_path(PLUGIN, BOT),
        ok(json!({"result": "no_model"})),
    );
    act(&rig, AgentModelsAction::TestModel);
    wait(&rig, |view| {
        matches!(view.test, Some(ModelTestState::Done { .. }))
    })
    .await;

    let current = agent_model_current_path(PLUGIN, BOT);
    rig.core.answer(
        Method::PUT,
        &current,
        ok(json!({"current": null, "confirm": "这个模型每百万字 15 美元"})),
    );
    act(
        &rig,
        AgentModelsAction::SelectModel {
            model: "openai/gpt".into(),
        },
    );
    let view = wait(&rig, |view| {
        view.provider
            .as_ref()
            .is_some_and(|page| page.confirm.is_some())
    })
    .await;
    assert_eq!(view.provider.unwrap().confirm.unwrap().model, "openai/gpt");
    assert!(view.current.is_none());

    rig.core.answer(
        Method::PUT,
        &current,
        ok(json!({"current": {"provider": "openrouter", "model": "openai/gpt"}})),
    );
    act(&rig, AgentModelsAction::ConfirmModelSelection);
    let view = wait(&rig, |view| view.current.is_some()).await;
    assert_eq!(view.current.unwrap().model, "openai/gpt");
    assert!(view.test.is_none());
    let bodies: Vec<Value> = rig
        .core
        .seen(&Method::PUT, &current)
        .iter()
        .map(|seen| serde_json::from_str(&seen.body).unwrap())
        .collect();
    assert_eq!(bodies[0].get("confirmed"), None);
    assert_eq!(bodies[1]["confirmed"], true);
}

#[tokio::test]
async fn a_test_reports_what_the_provider_said() {
    let rig = opened(true).await;
    rig.core.answer(
        Method::POST,
        &agent_model_test_path(PLUGIN, BOT),
        ok(json!({"result": "rejected", "message": "key 失效了"})),
    );
    act(&rig, AgentModelsAction::TestModel);
    assert_eq!(rig.app.agent_models().test, Some(ModelTestState::Running));
    let view = wait(&rig, |view| {
        matches!(view.test, Some(ModelTestState::Done { .. }))
    })
    .await;
    let Some(ModelTestState::Done { result }) = view.test else {
        unreachable!()
    };
    assert_eq!(result.result, ModelTestResult::Rejected);
}

/// 核心分好的类别原样给壳；本地代理自己回的（没有错误体）按状态码算连不上。
#[tokio::test]
async fn failures_keep_their_reason() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = agent_models_path(PLUGIN, BOT);
    rig.core.answer(
        Method::GET,
        &path,
        core_error(503, "not_ready", "Bot 还没起来"),
    );
    act(&rig, AgentModelsAction::OpenAgentModels { bot: bot() });
    let view = wait(&rig, |view| {
        matches!(view.state, AgentModelsState::Failed { .. })
    })
    .await;
    let AgentModelsState::Failed { problem } = view.state else {
        unreachable!()
    };
    assert_eq!(problem.reason, AgentModelsReason::NotReady);
    assert_eq!(problem.message, "Bot 还没起来");

    rig.core
        .answer(Method::GET, &path, Canned(502, "bad gateway".into()));
    act(&rig, AgentModelsAction::OpenAgentModels { bot: bot() });
    let view = wait(&rig, |view| {
        matches!(&view.state, AgentModelsState::Failed { problem } if problem.reason == AgentModelsReason::Transport)
    })
    .await;
    assert_eq!(view.bot, Some(bot()));
}

/// 插件给的登录网址不是 https 时不交给 App 内的浏览器。
#[tokio::test]
async fn a_login_url_that_is_not_https_is_refused() {
    let rig = opened(false).await;
    act(
        &rig,
        AgentModelsAction::OpenModelProvider {
            provider: "nous".into(),
        },
    );
    rig.core.answer(
        Method::POST,
        &agent_model_login_start_path(PLUGIN, BOT, "nous"),
        ok(json!({
            "session": SESSION, "url": "javascript:alert(1)",
            "expires_in": 600, "interval": 1,
        })),
    );
    act(&rig, AgentModelsAction::StartModelLogin);
    let view = wait(&rig, |view| view.action_problem.is_some()).await;
    assert_eq!(
        view.action_problem.unwrap().reason,
        AgentModelsReason::Protocol
    );
    assert!(view.provider.unwrap().login.is_none());
}
