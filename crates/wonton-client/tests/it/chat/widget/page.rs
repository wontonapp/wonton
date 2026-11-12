//! 像壳一样托管 widget 页：官方 matrix-widget-api 跑在 Node 里（`tabs_bridge/fixtures/widget.mjs`），
//! 页面发的报文经 `tab_handle_message` 交给核心，`subscribe_tab_widget` 的每一段脚本原样注入页面。

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use wonton_client::{ClientCore, PlacedTab, TabRef};

use crate::tabs_bridge::driver::node_image;
use crate::tabs_bridge::support::environment;

pub struct WidgetPage {
    core: Arc<ClientCore>,
    page: PlacedTab,
    child: Child,
    input: std::sync::mpsc::Sender<String>,
    output: mpsc::UnboundedReceiver<Value>,
    container: String,
    pump: tokio::task::JoinHandle<()>,
}

impl WidgetPage {
    /// 载入页面（注入脚本先于页面脚本），等 matrix-widget-api 握手完、能力谈好。
    pub async fn open(
        core: Arc<ClientCore>,
        tab: TabRef,
        url: &str,
        capabilities: &[&str],
    ) -> (Self, Value) {
        let page = PlacedTab::Bar { tab };
        // 起容器之前拿：拿不到就失败，不留下没人收的容器。
        let script = core
            .tab_document_script(page.clone(), environment())
            .unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/tabs_bridge/fixtures");
        let container = format!("wonton-widget-sdk-{}", std::process::id());
        let mut child = Command::new("docker")
            .args([
                "run",
                "-i",
                "--network=none",
                "--name",
                &container,
                "--mount",
                &format!(
                    "type=bind,source={},target=/fixture,readonly",
                    fixtures.display()
                ),
                node_image().as_str(),
                "node",
                "/fixture/widget.mjs",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let (input, lines) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in lines {
                if writeln!(stdin, "{line}")
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::unbounded_channel();
        let from_page = sender.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value = serde_json::from_str(&line).unwrap_or_else(|_| json!({"stray": line}));
                if from_page.send(value).is_err() {
                    return;
                }
            }
            let _ = from_page.send(json!({"exited": true}));
        });
        input
            .send(json!({"script": script, "url": url, "capabilities": capabilities}).to_string())
            .unwrap();
        // 壳在页面开始加载后订阅；驱动这时已经起来了（地址就是它生成的）。
        let subscription = core
            .subscribe_tab_widget(page.clone())
            .expect("订阅 widget 页");
        let to_page = input.clone();
        let pump = tokio::spawn(async move {
            while let Some(script) = subscription.next().await {
                if to_page.send(json!({"script": script}).to_string()).is_err() {
                    return;
                }
            }
            let _ = sender.send(json!({"subscriptionEnded": true}));
        });
        let mut widget = Self {
            core,
            page,
            child,
            input,
            output,
            container,
            pump,
        };
        let ready = widget.receive(|value| value["ready"] == true).await;
        (widget, ready)
    }

    /// 等页面输出一行满足条件的，期间把页面发的报文交给核心（壳收到 `postMessage` 时做的事）。
    pub async fn receive(&mut self, wanted: impl Fn(&Value) -> bool) -> Value {
        let waiting = async {
            loop {
                let value = self.output.recv().await.expect("widget 页的输出断了");
                if let Some(message) = value["message"].as_str() {
                    let result = self.core.tab_handle_message(
                        self.page.clone(),
                        true,
                        value["origin"].as_str().unwrap().to_owned(),
                        message.to_owned(),
                        environment(),
                    );
                    // widget 报文没有直接的应答，应答经订阅回来。
                    assert_eq!(result.reply_script, None, "{message}");
                } else if value["exited"] == true {
                    panic!("widget 页的进程退出了");
                } else if wanted(&value) {
                    return value;
                } else {
                    panic!("widget 页输出了没等的：{value}");
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(120), waiting)
            .await
            .expect("widget 页没有应答")
    }

    /// 让页面经 matrix-widget-api 调一个方法，返回 `{result}` 或 `{error}`。
    pub async fn call(&mut self, command: &str, params: Value) -> Value {
        self.input
            .send(json!({"id": command, "command": command, "params": params}).to_string())
            .unwrap();
        self.receive(|value| value["id"] == command).await
    }

    /// 订阅结束（核心收掉了驱动）时返回。
    pub async fn ended(&mut self) {
        self.receive(|value| value["subscriptionEnded"] == true)
            .await;
    }
}

impl Drop for WidgetPage {
    fn drop(&mut self) {
        self.pump.abort();
        let _ = self
            .input
            .send(json!({"id": "exit", "command": "exit"}).to_string());
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container])
            .stdout(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
