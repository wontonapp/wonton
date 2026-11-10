use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wonton_client::{TabBridgeAction, TabEnvironment, TabPlatform, TabRef};

use super::support::PAGE_ORIGIN;
use crate::plugins::rig::Rig;

/// node：本地从 Docker Hub 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn node_image() -> String {
    wonton_test_images::image(
        "node:22@sha256:18a6d90fd60542269850c80ca03b29b1d9b14d6b221ac7ee8e35b0d33ef904bd",
    )
}

pub struct Driver<'a> {
    rig: &'a Rig,
    tab: &'a TabRef,
    pub environment: TabEnvironment,
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
    container: Option<String>,
    pub actions: Vec<TabBridgeAction>,
    pub initialized: bool,
}

impl<'a> Driver<'a> {
    pub fn start(rig: &'a Rig, tab: &'a TabRef, environment: TabEnvironment) -> Self {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/tabs_bridge/fixtures");
        let platform = match environment.platform {
            TabPlatform::Ios => "ios",
            TabPlatform::Android => "android",
        };
        // 显式选择本地 Node 供无容器的开发冒烟；CI 的默认路径必须用官方镜像。
        let local_node = std::env::var_os("WONTON_TABS_NODE");
        let container = local_node
            .is_none()
            .then(|| format!("wonton-tabs-sdk-{}-{platform}", std::process::id()));
        let mut command = if let Some(node) = local_node {
            let mut command = Command::new(node);
            command.arg(fixtures.join("sdk.mjs"));
            command
        } else {
            let mut command = Command::new("docker");
            command.args([
                "run",
                "-i",
                "--network=none",
                "--name",
                container.as_ref().unwrap(),
                "--mount",
                &format!(
                    "type=bind,source={},target=/fixture,readonly",
                    fixtures.display()
                ),
                node_image().as_str(),
                "sh",
                "-c",
                "node /fixture/sdk.mjs; status=$?; printf '{\"sdkExitCode\":%d}\\n' \"$status\"; exit \"$status\"",
            ]);
            command
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut driver = Self {
            rig,
            tab,
            environment,
            child,
            input,
            output,
            container,
            actions: Vec::new(),
            initialized: false,
        };
        let script = rig
            .app
            .tab_document_script(super::support::bar(tab), driver.environment.clone())
            .unwrap();
        driver.send(
            json!({"script":script, "platform":platform, "url":format!("{PAGE_ORIGIN}/feed")}),
        );
        driver
    }

    pub fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }

    pub fn receive(&mut self) -> Value {
        loop {
            let line = self
                .output
                .recv_timeout(Duration::from_secs(if self.initialized { 60 } else { 300 }))
                .expect("SDK did not reply");
            let value: Value = serde_json::from_str(&line).unwrap();
            if let Some(message) = value["message"].as_str() {
                let result = self.rig.app.tab_handle_message(
                    super::support::bar(self.tab),
                    true,
                    value["origin"].as_str().unwrap().to_owned(),
                    message.to_owned(),
                    self.environment.clone(),
                );
                self.initialized |= result.initialized;
                self.actions.extend(result.action);
                if let Some(script) = result.reply_script {
                    self.send(json!({"script":script}));
                }
            } else {
                return value;
            }
        }
    }

    pub fn command(&mut self, name: &str, params: Value) -> Value {
        self.send(json!({"id":name, "command":name, "params":params}));
        let reply = self.receive();
        assert_eq!(reply["id"], name);
        reply
    }

    pub fn update_context(&mut self) {
        let script = self
            .rig
            .app
            .tab_context_script(super::support::bar(self.tab), self.environment.clone())
            .unwrap();
        self.send(json!({"script":script}));
    }

    pub fn finish(mut self) {
        assert_eq!(self.command("exit", Value::Null)["closed"], true);
        if self.container.is_some() {
            let exit = self
                .output
                .recv_timeout(Duration::from_secs(10))
                .expect("SDK did not exit");
            assert_eq!(
                serde_json::from_str::<Value>(&exit).unwrap(),
                json!({"sdkExitCode": 0})
            );
        } else {
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.child.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "SDK did not exit");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(self.child.wait().unwrap().success());
        if let Some(name) = &self.container {
            assert!(
                Command::new("docker")
                    .args(["rm", name])
                    .status()
                    .unwrap()
                    .success(),
                "SDK container cleanup failed"
            );
            self.container = None;
        }
    }
}

impl Drop for Driver<'_> {
    fn drop(&mut self) {
        if let Some(name) = &self.container {
            let _ = Command::new("docker").args(["rm", "-f", name]).status();
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
