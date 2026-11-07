//! Agent 写插件的只读工具（ADR-0033）：说明带两个能过校验的例子、outbox 目录的校验、插件状态和网页探测。
//! 都不请主人确认。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use super::tests::{Setup, failed, setup, text};
use crate::plugin::PluginId;
use crate::plugin::docker::Engine as Docker;
use crate::plugin::languages::Languages;
use crate::plugin::manager::PluginStatus;

const GUIDE: &str = "wonton__plugin_guide";
const CHECK: &str = "wonton__check_plugin";
const STATUS: &str = "wonton__plugin_status";

struct Bench {
    s: Setup,
    outbox: PathBuf,
}

async fn bench() -> Bench {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    let id = PluginId::parse("com.example.agent").unwrap();
    let outbox = s.bus.plugins.app_data_dir(&id).join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    Bench { s, outbox }
}

impl Bench {
    fn write(&self, dir: &str, path: &str, content: &str) {
        let path = self.outbox.join(dir).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn result(&self, result: &CallToolResult) -> Value {
        serde_json::from_str(&text(result)).unwrap()
    }
}

/// 例子来自 wonton-docs 的 `skills/wonton-plugin/`（编进了二进制），两个都必须能过 `check_plugin`
/// 的同一份校验。在那边改了规范，这里的断言会拦住没同步的核心。
#[tokio::test]
async fn both_examples_in_the_guide_pass_the_check() {
    let b = bench().await;
    let guide = b.result(&b.s.call(GUIDE, json!({})).await);
    assert!(!guide["spec_version"].is_null(), "{guide}");
    assert!(
        guide["guide"]
            .as_str()
            .unwrap()
            .contains("docker-compose.yml")
    );
    let examples = guide["examples"].as_object().unwrap();
    assert_eq!(examples.len(), 2, "{guide}");
    for (name, files) in examples {
        for file in files.as_array().unwrap() {
            let path = file["path"].as_str().unwrap();
            let content = file["content"].as_str().unwrap();
            assert!(
                content.contains("init: true") || !path.ends_with(".yml"),
                "{name}/{path} 的例子要写 init: true"
            );
            b.write(name, path, content);
        }
        let checked = b.result(&b.s.call(CHECK, json!({"dir": name})).await);
        assert!(
            checked["problems"].as_array().unwrap().is_empty(),
            "{name} 过不了校验：{checked}"
        );
        assert!(!checked["digest"].is_null(), "{checked}");
    }
}

#[tokio::test]
async fn a_valid_directory_is_reported_without_touching_it() {
    let b = bench().await;
    let manifest = r#"name: app
services:
  web:
    image: nginx:1.29-alpine
    init: true
    configs:
      - {source: page, target: /usr/share/nginx/html/index.html}
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://127.0.0.1/"]
configs:
  page: {file: index.html}
x-wonton:
  spec: '0.5'
  id: app.example.hello
  version: 0.1.0
  kind: app
  title: {zh: 你好, en: Hello}
  description: {zh: 一个静态页面, en: A static page}
  webui: {service: web, port: 80}
  permissions: []
"#;
    b.write("myapp", "docker-compose.yml", manifest);
    b.write("myapp", "index.html", "<h1>你好</h1>");

    let checked = b.s.call(CHECK, json!({"dir": "myapp"})).await;
    assert!(!failed(&checked), "{checked:?}");
    let checked = b.result(&checked);
    assert_eq!(checked["problems"], json!([]));
    let digest = checked["digest"].as_str().unwrap();
    assert!(digest.starts_with("sha256:"));
    let files: Vec<&str> = checked["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["docker-compose.yml", "index.html"]);

    // 和 wonton__save_file 不同：核心读完不删目录，Agent 接着改。
    assert!(b.outbox.join("myapp/docker-compose.yml").exists());
    assert_eq!(
        fs::read_to_string(b.outbox.join("myapp/index.html")).unwrap(),
        "<h1>你好</h1>"
    );

    // 改一个文件再校验，摘要跟着变。
    b.write("myapp", "index.html", "<h1>改了</h1>");
    let again = b.result(&b.s.call(CHECK, json!({"dir": "myapp"})).await);
    assert_ne!(again["digest"].as_str().unwrap(), digest);
}

/// 校验不过、清单解析不了都是正常结果，问题列在 `problems` 里，不是工具错误。
#[tokio::test]
async fn every_problem_is_listed_instead_of_failing_the_call() {
    let b = bench().await;
    let manifest = r#"name: app
services:
  web:
    image: nginx
    init: true
x-wonton:
  spec: '0.5'
  id: app.example.bad
  version: 0.1.0
  kind: app
  title: {zh: 坏, en: Bad}
  description: {zh: 坏的, en: Bad}
  icon: icon.svg
  webui: {service: web, port: 80}
  permissions: [tools]
  tabs:
    - {id: home, title: {zh: 坏, en: Bad}, icon: tabs/home.svg, path: /}
"#;
    b.write("bad", "docker-compose.yml", manifest);
    b.write("bad", "icon.svg", "不是 SVG 的内容");
    b.write("bad", "tabs/home.svg", "也不是 SVG 的内容");

    let checked = b.s.call(CHECK, json!({"dir": "bad"})).await;
    assert!(!failed(&checked), "{checked:?}");
    let checked = b.result(&checked);
    let problems = checked["problems"].as_array().unwrap();
    assert!(problems.len() >= 2, "{problems:?}");
    let joined = problems
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("tabs[0].icon"), "{joined}");

    // 清单读不了也一样：正常返回，原因在 problems 里。
    b.write("worse", "docker-compose.yml", "\tx-wonton: [");
    let checked = b.result(&b.s.call(CHECK, json!({"dir": "worse"})).await);
    assert!(
        checked["problems"].as_array().unwrap().len() == 1,
        "{checked}"
    );
    assert!(checked["digest"].is_null(), "{checked}");
}

/// 路径规则和 `wonton__save_file` 同一套：`..`、软链接、目录不存在都被拒，软链接是权限问题，
/// 其余是输入不合法。
#[tokio::test]
async fn paths_that_leave_the_outbox_are_refused() {
    let b = bench().await;
    for (dir, kind) in [
        ("../escape", "invalid_input"),
        ("ghost", "invalid_input"),
        ("a//b", "invalid_input"),
    ] {
        let result = b.s.call(CHECK, json!({"dir": dir})).await;
        assert!(failed(&result), "{dir}: {result:?}");
        assert!(text(&result).contains(kind), "{dir}: {result:?}");
    }

    b.write("real", "docker-compose.yml", "name: app");
    #[cfg(unix)]
    std::os::unix::fs::symlink(b.outbox.join("real"), b.outbox.join("alias")).unwrap();
    let result = b.s.call(CHECK, json!({"dir": "alias"})).await;
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("permission_denied"), "{result:?}");
}

/// 没挂 outbox 的 Bot 报的是权限问题，不是“目录不存在”。
#[tokio::test]
async fn a_bot_without_an_outbox_is_told_so() {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    let result = s.call(CHECK, json!({"dir": "myapp"})).await;
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("permission_denied"), "{result:?}");
}

/// 状态：装了的报状态和网页探测的状态码；没装的是 `invalid_input`，不是工具的锅。
#[tokio::test]
async fn status_reports_the_running_plugin_and_refuses_unknown_ones() {
    let b = bench().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/",
        axum::routing::get(|| async { axum::http::StatusCode::OK }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let manifest = manifest_of("app.example.hello");
    b.s.manager
        .register_for_test(manifest, PluginStatus::Running, Some(format!("{address}")));

    let status = b.result(&b.s.call(STATUS, json!({"id": "app.example.hello"})).await);
    assert_eq!(status["status"]["state"], "running", "{status}");
    assert_eq!(status["web_status"], 200, "{status}");
    assert!(status["web_error"].is_null(), "{status}");
    assert!(b.s.cards.posted.lock().unwrap().is_empty(), "不发卡片");

    // 探测不到时如实说，不编一个状态码。
    let manifest = manifest_of("app.example.down");
    b.s.manager.register_for_test(
        manifest,
        PluginStatus::Running,
        Some("127.0.0.1:1".to_owned()),
    );
    let status = b.result(&b.s.call(STATUS, json!({"id": "app.example.down"})).await);
    assert!(status["web_status"].is_null(), "{status}");
    assert!(
        !status["web_error"].as_str().unwrap().is_empty(),
        "{status}"
    );

    let result = b.s.call(STATUS, json!({"id": "app.example.missing"})).await;
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("invalid_input"), "{result:?}");
}

fn manifest_of(id: &str) -> crate::plugin::manifest::Manifest {
    crate::plugin::PluginPackage::parse(
        &format!(
            r#"name: app
services:
  web:
    image: nginx
x-wonton:
  spec: '0.5'
  id: {id}
  version: 0.1.0
  kind: app
  title: {{zh: 页, en: Page}}
  description: {{zh: 页面, en: A page}}
  webui: {{service: web, port: 80}}
  permissions: []
"#
        ),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap()
    .manifest
}

/// 三个工具都在列表里，都标只读：不进确认流程（ADR-0033 的表里都不是敏感工具）。
#[tokio::test]
async fn the_three_tools_are_listed_and_read_only() {
    let b = bench().await;
    let tools = b.s.bus.tools(&b.s.caller).await;
    for name in [GUIDE, CHECK, STATUS] {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == name)
            .unwrap_or_else(|| panic!("没有 {name}"));
        assert_eq!(
            tool.annotations.as_ref().and_then(|a| a.read_only_hint),
            Some(true),
            "{name}"
        );
    }
    for name in [GUIDE, CHECK, STATUS] {
        let _ =
            b.s.call(name, json!({"dir": "x", "id": "app.example.x"}))
                .await;
    }
    assert!(b.s.cards.posted.lock().unwrap().is_empty(), "不发卡片");
}
