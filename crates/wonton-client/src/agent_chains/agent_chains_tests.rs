//! 行为测试：给假核心按顺序回准备好的响应，断言列表和动作的结果（ADR-0061 第 10 节的完成标准）。

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use wonton_protocol::{
    AgentChainAction, AgentChainStatus, ChainLimits, ChainSpend, CoreError, ProxySettings,
};

use super::actions::{self, Effect, perform};
use super::{apply_row, fetch_list};

/// 假核心按顺序回的每一步：请求行要打中给的方法和路径片段，PUT 的请求体要带上给的内容。
struct Step {
    method: &'static str,
    path: &'static str,
    body_has: &'static [&'static str],
    status: u16,
    body: String,
}

fn step(
    method: &'static str,
    path: &'static str,
    status: u16,
    body: Value,
    body_has: &'static [&'static str],
) -> Step {
    Step {
        method,
        path,
        body_has,
        status,
        body: body.to_string(),
    }
}

/// 起假核心：每个连接取走下一步，方法或路径对不上就断掉，测试在那头失败。回应完关连接。
async fn serve(steps: Vec<Step>) -> (url::Url, ProxySettings) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let steps = Arc::new(Mutex::new(steps));
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let step = {
                let mut queue = steps.lock().unwrap();
                if queue.is_empty() {
                    panic!("the fake core got more requests than scripted");
                }
                queue.remove(0)
            };
            let head = read_head(&mut tcp).await;
            let request_line = head.lines().next().unwrap().to_owned();
            let (method, target) = request_line.split_once(' ').unwrap();
            assert_eq!(method, step.method, "the fake core got {request_line}");
            assert!(
                target.contains(step.path),
                "the fake core got {request_line}"
            );
            if let Some(length) = content_length(&head) {
                let mut body = vec![0; length];
                tcp.read_exact(&mut body).await.unwrap();
                let body = String::from_utf8(body).unwrap();
                for piece in step.body_has {
                    assert!(body.contains(piece), "the request body was {body}");
                }
            }
            let response = format!(
                "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                step.status,
                step.body.len(),
                step.body
            );
            tcp.write_all(response.as_bytes()).await.unwrap();
            tcp.flush().await.unwrap();
        }
    });
    let proxy = ProxySettings {
        host: Ipv4Addr::LOCALHOST.to_string(),
        port,
        username: "phone".to_owned(),
        password: "secret".to_owned(),
        domain: "wonton.localhost".to_owned(),
    };
    (url::Url::parse("http://wonton.localhost").unwrap(), proxy)
}

async fn read_head(tcp: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        tcp.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

fn content_length(head: &str) -> Option<usize> {
    head.lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split_once(':'))
        .and_then(|(_, value)| value.trim().parse().ok())
}

fn chain_row(status: &str, spend: Value) -> Value {
    json!({
        "chain": "c-1",
        "user": "@alex:home.example",
        "origin": {"room_id": "!r:home.example", "thread_id": null},
        "bots": [{"bot": {"plugin_id": "app.orchestrator", "bot": "owner"}, "layer": 1}],
        "rounds": 2,
        "spend": spend,
        "status": status,
        "can_stop": true,
    })
}

fn chains_view(row: Value) -> Value {
    json!({"chains": [row], "revision": 1})
}

async fn fetch(
    core: &url::Url,
    proxy: &ProxySettings,
    after: Option<u64>,
) -> super::AgentChainsViewModel {
    fetch_list(core, proxy, after)
        .await
        .unwrap_or_else(|failure| panic!("the list fetch failed: {}", failure.describe()))
}

/// 行为：运行中的链，停链动作成功后核心把停后的整行给回来，列表里那一行变成 Stopped。
#[tokio::test]
async fn stopping_a_running_chain_shows_it_stopped() {
    let (core, proxy) = serve(vec![
        step(
            "GET",
            "/_wonton/api/agent/chains",
            200,
            chains_view(chain_row(
                "running",
                json!({"type": "via_gateway", "tokens": 12}),
            )),
            &[],
        ),
        step(
            "POST",
            "/_wonton/api/agent/chains/c-1/stop",
            200,
            chain_row("stopped", json!({"type": "via_gateway", "tokens": 12})),
            &[],
        ),
    ])
    .await;

    let view = fetch(&core, &proxy, None).await;
    assert_eq!(view.chains[0].status, AgentChainStatus::Running);

    let effect = perform(
        &core,
        &proxy,
        &AgentChainAction::StopChain {
            chain: "c-1".to_owned(),
        },
    )
    .await
    .unwrap();
    let Effect::Stopped(row) = effect else {
        panic!("stopping must hand back the stopped row");
    };

    let mut view = view;
    apply_row(&mut view, row);
    assert_eq!(view.chains[0].status, AgentChainStatus::Stopped);
    assert_eq!(view.revision, 1);
}

/// 行为：比壳新的核心给出的不认识的状态落到 Other，整份列表照常交出去。
#[tokio::test]
async fn an_unknown_status_falls_back_to_other() {
    let (core, proxy) = serve(vec![step(
        "GET",
        "/_wonton/api/agent/chains",
        200,
        chains_view(chain_row("waking", json!({"type": "unknown"}))),
        &[],
    )])
    .await;
    let view = fetch(&core, &proxy, None).await;
    assert_eq!(view.chains[0].status, AgentChainStatus::Other);
}

/// 行为：花费记不上时核心给 unknown，列表里就是 Unknown。
#[tokio::test]
async fn spend_the_core_could_not_record_is_unknown() {
    let (core, proxy) = serve(vec![step(
        "GET",
        "/_wonton/api/agent/chains",
        200,
        chains_view(chain_row("running", json!({"type": "unknown"}))),
        &[],
    )])
    .await;
    let view = fetch(&core, &proxy, None).await;
    assert_eq!(view.chains[0].spend, ChainSpend::Unknown);
}

/// 行为：停别人的链被核心拒绝（403）时，动作带类别失败，手里的列表不动。
#[tokio::test]
async fn a_forbidden_stop_fails_with_a_category() {
    let (core, proxy) = serve(vec![
        step(
            "GET",
            "/_wonton/api/agent/chains",
            200,
            chains_view(chain_row("running", json!({"type": "unknown"}))),
            &[],
        ),
        step(
            "POST",
            "/_wonton/api/agent/chains/c-1/stop",
            403,
            json!({"error": {"kind": "not_yours", "message": "这条链不归你"}}),
            &[],
        ),
    ])
    .await;
    let view = fetch(&core, &proxy, None).await;

    let error = perform(
        &core,
        &proxy,
        &AgentChainAction::StopChain {
            chain: "c-1".to_owned(),
        },
    )
    .await
    .unwrap_err();
    match error {
        CoreError::Forbidden { reason } => assert!(reason.contains("这条链不归你"), "{reason}"),
        error => panic!("403 must come back as Forbidden, got {error:?}"),
    }

    assert_eq!(view.chains[0].status, AgentChainStatus::Running);
}

/// 行为：改上限把上限发给核心，回应里的新设置值解析得回来。
#[tokio::test]
async fn setting_agent_limits_sends_them_and_reads_the_new_settings() {
    let (core, proxy) = serve(vec![step(
        "PUT",
        "/_wonton/api/agent/limits",
        200,
        json!({"limits": {"max_depth": 5, "max_rounds": 100, "max_tokens": 4000000}}),
        &["\"max_depth\":5"],
    )])
    .await;
    let effect = perform(
        &core,
        &proxy,
        &AgentChainAction::SetAgentLimits {
            limits: ChainLimits {
                max_depth: 5,
                max_rounds: 100,
                max_tokens: 4_000_000,
            },
        },
    )
    .await
    .unwrap();
    assert!(matches!(effect, Effect::Done));
}

/// 行为：改使用方并发把插件 ID 和数字发给核心，回应里的数值解析得回来。
#[tokio::test]
async fn setting_consumer_concurrency_sends_the_plugin_and_the_number() {
    let (core, proxy) = serve(vec![step(
        "PUT",
        "/_wonton/api/agent/consumers/app.orchestrator/concurrency",
        200,
        json!({"concurrency": 8}),
        &["\"concurrency\":8"],
    )])
    .await;
    let effect = perform(
        &core,
        &proxy,
        &AgentChainAction::SetConsumerAgentConcurrency {
            plugin_id: "app.orchestrator".to_owned(),
            concurrency: 8,
        },
    )
    .await
    .unwrap();
    assert!(matches!(effect, Effect::Done));
}

/// 行为：设置页取当前上限。
#[tokio::test]
async fn agent_settings_reads_the_current_limits() {
    let (core, proxy) = serve(vec![step(
        "GET",
        "/_wonton/api/agent/settings",
        200,
        json!({"limits": {"max_depth": 3, "max_rounds": 50, "max_tokens": 2000000}}),
        &[],
    )])
    .await;
    let settings = actions::settings(&core, &proxy).await.unwrap();
    assert_eq!(settings.limits, ChainLimits::default());
}
