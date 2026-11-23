//! USR2 返回只说明 Docker 收到了信号；新 worker 接管后才算重载完成。

use std::time::Duration;

use super::{ENTRY_COMPONENT, EntryContainers, EntryError, config_error};
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

impl EntryContainers {
    pub(super) async fn reload_checked(&self) -> Result<(), EntryError> {
        let found = self
            .engine
            .labeled(LABEL_MANAGED_BY, "core", |labels| {
                self.instance.owns(labels)
                    && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(ENTRY_COMPONENT)
            })
            .await?;
        let [name] = found.containers.as_slice() else {
            return Err(EntryError::NotOneEntry(found.containers.len()));
        };
        let check = self
            .engine
            .exec(
                name,
                &["haproxy", "-c", "-f", "/config/haproxy.cfg"].map(str::to_owned),
                None,
                Duration::from_secs(10),
            )
            .await?;
        if check.exit_code != 0 {
            return Err(config_error(
                "校验入口配置",
                anyhow::anyhow!("HAProxy 退出码 {}：{}", check.exit_code, check.stderr),
            ));
        }
        let previous = self.wait_worker(name, None).await?;
        self.engine.kill_container(name, "SIGUSR2").await?;
        self.wait_worker(name, Some(previous)).await.map(|_| ())
    }

    async fn wait_worker(&self, name: &str, previous: Option<u32>) -> Result<u32, EntryError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let result = self.worker_pid(name).await;
            if let Ok(pid) = &result
                && Some(*pid) != previous
            {
                return Ok(*pid);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(result.err().unwrap_or_else(|| {
                    config_error("重载入口配置", anyhow::anyhow!("HAProxy 新 worker 未接管"))
                }));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn worker_pid(&self, name: &str) -> Result<u32, EntryError> {
        let command = format!(
            "printf 'show info\\n' | nc -w 2 {}",
            crate::entry_haproxy::RUNTIME_SOCKET.replace(':', " ")
        );
        let output = self
            .engine
            .exec_output(
                name,
                &["sh".to_owned(), "-c".to_owned(), command],
                None,
                Duration::from_secs(3),
            )
            .await?;
        let pid = output.stdout.lines().find_map(|line| {
            line.strip_prefix("Pid: ")
                .and_then(|value| value.trim().parse::<u32>().ok())
        });
        match (output.exit_code, pid) {
            (0, Some(pid)) if pid > 0 => Ok(pid),
            _ => Err(config_error(
                "读取入口运行状态",
                anyhow::anyhow!(
                    "HAProxy 未返回 worker PID（退出码 {}）：{}",
                    output.exit_code,
                    output.stderr
                ),
            )),
        }
    }
}
