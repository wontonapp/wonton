//! 一份消费方契约：连接器只需实现 probe、配置头和清理接口，不依赖具体提供方的包。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use serde_json::json;
use tracing::instrument::WithSubscriber;
use wonton_protocol::{PluginHintCategory, PluginState};

use crate::plugin::tests::langs;

pub(crate) mod fixture;
use fixture::{Bed, PROVIDER, REMOTE, SECRET, TEMPLATE, id, remote};

#[tokio::test]
async fn install_remote_mcp_lists_tools_on_bus() {
    let mut bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let servers = bed.manager.mcp_servers();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].tool_limits.idle, Duration::from_secs(47));
    assert_eq!(servers[0].tool_limits.total, Duration::from_secs(91));
    assert!(!format!("{:?}", servers[0]).contains(SECRET));
    let agent = bed.agent().await;
    let tools = agent.list_tools(None).await.unwrap().tools;
    let expected = format!("{}__search", id(REMOTE).slug());
    assert!(tools.iter().any(|tool| tool.name == expected));
    assert!(!tools.iter().any(|tool| tool.name.ends_with("__hidden")));
    let result = agent
        .call_tool(CallToolRequestParams::new(expected))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert!(
        serde_json::to_string(&result)
            .unwrap()
            .contains("remote-called")
    );
    assert_eq!(bed.docker.containers(), [bed.provider_container()]);
    let audit = std::fs::read_to_string(bed.data.path().join("audit.jsonl")).unwrap();
    assert!(!audit.contains(SECRET));
    assert!(!audit.contains("https://mcp.example.com/mcp/old%2Fone%3Fx%3D1"));
    agent.cancel().await.unwrap();
}

#[tokio::test]
async fn bus_requests_carry_remote_headers() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let upstreams = crate::mcp::upstream::Upstreams::new();
    let server = bed.manager.mcp_servers().pop().unwrap();
    upstreams.tools(&server).await.unwrap();
    upstreams
        .call(&server, "search", None, std::future::pending())
        .await
        .unwrap();
    let calls = bed.fake.calls.lock().unwrap();
    assert!(!calls.is_empty());
    for (uri, headers) in calls.iter() {
        assert_eq!(headers["authorization"], format!("Bearer {SECRET}"));
        assert!(
            headers
                .keys()
                .all(|name| !name.as_str().starts_with("x-wonton-"))
        );
        assert!(uri.contains("old%2Fone%3Fx%3D1"), "{uri}");
    }
    let probes = bed.fake.probes.lock().unwrap();
    assert_eq!(probes[0]["auth"], "header");
    assert_eq!(
        probes[0]["headers"]["authorization"],
        format!("Bearer {SECRET}")
    );
    assert_eq!(
        probes[0]["url"],
        "https://mcp.example.com/mcp/old%2Fone%3Fx%3D1?key=old%2Fone%3Fx%3D1"
    );
}

#[tokio::test]
async fn secret_change_applies_to_next_request() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let upstreams = crate::mcp::upstream::Upstreams::new();
    let before = bed.manager.mcp_servers().pop().unwrap();
    upstreams
        .call(&before, "search", None, std::future::pending())
        .await
        .unwrap();
    let writes = bed.docker.writes();
    let fresh = "new/two?y=2";
    let detail = bed
        .manager
        .set_secret_input(&id(REMOTE), "token", Some(fresh), &langs())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&detail).unwrap().contains(fresh));
    let after = bed.manager.mcp_servers().pop().unwrap();
    assert_ne!(before.headers, after.headers);
    upstreams
        .call(&after, "search", None, std::future::pending())
        .await
        .unwrap();
    let calls = bed.fake.calls.lock().unwrap();
    let (uri, headers) = calls.last().unwrap();
    assert_eq!(headers["authorization"], format!("Bearer {fresh}"));
    assert!(uri.contains("new%2Ftwo%3Fy%3D2"), "{uri}");
    assert_eq!(bed.docker.writes(), writes, "改密钥不能重建任何容器");
}

#[tokio::test]
async fn install_without_provider_reports_missing_requires() {
    let bed = Bed::new(false).await;
    let preview = bed
        .manager
        .preview_install(&remote("header"), &langs())
        .await
        .unwrap();
    assert_eq!(preview.missing_requires.len(), 1);
    assert_eq!(preview.missing_requires[0].capability, "remote_mcp");
    assert!(preview.missing_requires[0].providers.is_empty());
    assert!(bed.install("header").await.is_err());
    assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
}

#[tokio::test]
async fn provider_stopped_marks_dependency_not_running() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    bed.docker.set_state(
        &bed.provider_container(),
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    bed.manager.watch_containers().await;
    let stopped = bed.manager.installed_detail(&id(REMOTE), &langs()).unwrap();
    assert!(matches!(
        stopped.installed.as_ref().unwrap().status,
        PluginState::Degraded { .. }
    ));
    assert!(stopped.hints.iter().any(|hint| {
        hint.category == PluginHintCategory::DependencyStopped
            && hint
                .provider
                .as_ref()
                .is_some_and(|provider| provider.id == PROVIDER)
    }));
    assert!(bed.manager.mcp_servers().is_empty());
    bed.docker.set_state(
        &bed.provider_container(),
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    bed.manager.watch_containers().await;
    let running = bed.manager.installed_plugin(&id(REMOTE), &langs()).unwrap();
    assert!(matches!(running.status, PluginState::Running { .. }));
    assert_eq!(bed.manager.mcp_servers().len(), 1);
}

#[tokio::test]
async fn uninstall_provider_blocked_by_remote_plugins() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let error = bed
        .manager
        .uninstall(&id(PROVIDER), false, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
    assert!(error.to_string().contains("Remote tools"));
    assert!(bed.manager.record_for_test(&id(PROVIDER)).is_some());
}

#[tokio::test]
async fn uninstall_remote_mcp_asks_connector_to_forget() {
    for status in [
        hyper::StatusCode::NO_CONTENT,
        hyper::StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let bed = Bed::new(true).await;
        bed.install("header").await.unwrap();
        *bed.fake.forget_status.lock().unwrap() = status;
        let report = bed
            .manager
            .uninstall(&id(REMOTE), true, &langs())
            .await
            .unwrap();
        assert_eq!(
            *bed.fake.forgotten.lock().unwrap(),
            [format!("/_wonton/plugins/{REMOTE}")]
        );
        assert_eq!(report.warnings.is_empty(), status.is_success());
        assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
        assert!(bed.manager.mcp_servers().is_empty());
    }
}

#[tokio::test]
async fn legacy_sse_probe_installs_and_forwards_tools() {
    let bed = Bed::new(true).await;
    *bed.fake.outcome.lock().unwrap() = "legacy_sse".into();
    bed.install("none").await.unwrap();
    let server = bed.manager.mcp_servers().pop().unwrap();
    let upstreams = crate::mcp::upstream::Upstreams::new();
    assert_eq!(upstreams.tools(&server).await.unwrap()[0].name, "search");
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn expanded_remote_url_never_enters_errors_details_or_logs() {
    let bed = Bed::new(true).await;
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || LogWriter(writer.clone()))
        .finish();
    *bed.fake.outcome.lock().unwrap() = "secret_rejected".into();
    let error = bed
        .install("header")
        .with_subscriber(subscriber)
        .await
        .unwrap_err();
    let details = error.to_string();
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    for output in [details.as_str(), logs.as_str()] {
        assert!(output.contains(TEMPLATE), "{output}");
        assert!(!output.contains(SECRET), "{output}");
        assert!(!output.contains("old%2Fone%3Fx%3D1"), "{output}");
    }
    assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
    let request = &bed.fake.probes.lock().unwrap()[0];
    assert!(
        request["url"]
            .as_str()
            .unwrap()
            .contains("old%2Fone%3Fx%3D1")
    );
}

#[tokio::test]
async fn remote_oauth_install_requires_authorization_before_probe() {
    let bed = Bed::new(true).await;
    let error = bed.install("oauth").await.unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(bed.fake.probes.lock().unwrap().is_empty());
    assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
}

#[tokio::test]
async fn remote_mcp_install_confirms_connector_together() {
    let bed = Bed::new(false).await;
    bed.publish_provider();
    let preview = bed
        .manager
        .preview_upload(remote("header"), &langs())
        .await
        .unwrap();
    let candidate = &preview.missing_requires[0].providers[0];
    let provider = bed
        .manager
        .preview_source(&candidate.source, &langs())
        .await
        .unwrap();
    let request = serde_json::from_value(json!({
        "source": preview.source, "digest": preview.digest, "plan": preview.plan, "grant": [],
        "secrets": {"token": "request/key"},
        "install_together": [{"id": candidate.id, "request": {
            "source": provider.source, "digest": provider.digest, "plan": provider.plan, "grant": []
        }}]
    }))
    .unwrap();
    bed.manager.install_source(&request).await.unwrap();
    assert!(bed.manager.record_for_test(&id(PROVIDER)).is_some());
    assert!(bed.manager.record_for_test(&id(REMOTE)).is_some());
    assert_eq!(
        bed.fake.probes.lock().unwrap()[0]["headers"]["authorization"],
        "Bearer request/key"
    );
    assert_eq!(bed.manager.mcp_servers().len(), 1);
}

#[tokio::test]
async fn remote_mcp_probe_failure_restores_submitted_secrets() {
    let bed = Bed::new(true).await;
    *bed.fake.outcome.lock().unwrap() = "remote_error".into();
    let preview = bed
        .manager
        .preview_upload(remote("header"), &langs())
        .await
        .unwrap();
    let request = serde_json::from_value(json!({
        "source": preview.source, "digest": preview.digest, "plan": preview.plan,
        "grant": [], "secrets": {"token": "request/secret"}
    }))
    .unwrap();
    let error = bed.manager.install_source(&request).await.unwrap_err();
    assert_eq!(error.kind(), "remote_error");
    assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
    assert_eq!(
        bed.manager
            .store
            .secrets()
            .load(&id(REMOTE))
            .unwrap()
            .unwrap()["token"],
        SECRET.as_bytes()
    );
    assert_eq!(
        bed.fake.probes.lock().unwrap()[0]["headers"]["authorization"],
        "Bearer request/secret"
    );
}

#[tokio::test]
async fn remote_mcp_restores_without_a_directory_or_another_probe() {
    let bed = Bed::new(true).await;
    bed.install("header").await.unwrap();
    let restarted = super::manager::manager_with(bed.docker.engine().await, &bed.data);
    restarted.restore().await.unwrap();
    assert!(matches!(
        restarted
            .installed_plugin(&id(REMOTE), &langs())
            .unwrap()
            .status,
        PluginState::Running { .. }
    ));
    assert_eq!(restarted.mcp_servers().len(), 1);
    assert_eq!(bed.fake.probes.lock().unwrap().len(), 1);
}
