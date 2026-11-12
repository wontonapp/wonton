//! 联邦名单和 hosts 由核心生成和更新（ADR-0050）。
//!
//! 输入是已配对服务器名的 `tokio::sync::watch::Receiver<BTreeSet<String>>`（配对组件给出）。名单变了：
//! 原地重写安装目录 `fed/` 里的 `config.toml`（名单）和 `hosts`（每个名字指向核心的固定地址），再经
//! Docker API 给 Tuwunel 发 `SIGUSR1`。两份文件都是单文件绑定挂载，容器钉住挂载时的 inode，所以只做
//! 原地截断写，不用临时文件改名。hosts 不能热加载（Tuwunel 只在启动时读它，`SIGUSR1` 不重建解析器，
//! 实测见 ADR-0050 的“验证”）：名单新增了名字就重启 Tuwunel；只删名字时 `SIGUSR1` 就够，被删名字的
//! 出站已被名单挡住，不再需要解析它。
//!
//! 失败如实记日志，并经通知房间告诉主人；不吞掉。

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::watch;

use super::NoticeRoom;
use crate::plugin::docker::{Engine, LabeledResources};

mod files;
#[cfg(test)]
mod tests;

pub(crate) use files::{allowlist_toml, hosts, names_in_hosts};

/// 核心在联邦网络上的默认固定地址：hosts 名单都指向它。正式安装由
/// `WONTON_FEDERATION_CORE_ADDR` 配置（compose 和 install.sh 写进 .env），同机装多套时
/// 每套一段；和 `deploy/install/compose.yaml` 里 federation 网络的 subnet 是一对，一起改。
pub(crate) const DEFAULT_CORE_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 212, 0, 10));

/// Compose 项目里 Tuwunel 容器的服务名标签。
const SERVICE_LABEL: &str = "com.docker.compose.service";

/// 盯着配对名单，变了就把 Tuwunel 的配置跟上来。
pub struct FederationConfig {
    engine: Engine,
    /// 安装目录，`fed/config.toml` 和 `fed/hosts` 由这里重写。
    install_dir: PathBuf,
    /// 核心自己的容器名，用它找同一个 Compose 项目里的 Tuwunel。
    core_container: Option<String>,
    /// 核心在联邦网络上的地址，hosts 的条目都指向它。
    core_addr: IpAddr,
    notice: Option<Arc<NoticeRoom>>,
}

impl FederationConfig {
    /// 宿主上直接跑核心时（没有安装目录或容器名）什么都不做：联邦只在正式安装里生效。
    pub fn spawn(
        engine: Engine,
        install_dir: Option<PathBuf>,
        core_container: Option<String>,
        core_addr: IpAddr,
        notice: Option<Arc<NoticeRoom>>,
        mut peers: watch::Receiver<BTreeSet<String>>,
    ) {
        let (Some(install_dir), Some(core_container)) = (install_dir, core_container) else {
            tracing::debug!("没有安装目录或核心容器名，联邦名单交给 Tuwunel 的安装配置");
            return;
        };
        let config = Arc::new(Self {
            engine,
            install_dir,
            core_container: Some(core_container),
            core_addr,
            notice,
        });
        tokio::spawn(async move {
            // 启动时先把名单对齐（配对组件还没合时集合一直是空的，写出来的和安装时一样）。
            let initial = peers.borrow_and_update().clone();
            config.apply(initial).await;
            while peers.changed().await.is_ok() {
                let names = peers.borrow_and_update().clone();
                config.apply(names).await;
            }
            // 发送方没了：名单停在最后的值上，如实记一条。
            tracing::warn!("联邦名单的来源已关闭，名单停在最后的值");
        });
    }

    async fn apply(&self, names: BTreeSet<String>) {
        if let Err(error) = self.apply_inner(&names).await {
            tracing::error!(%error, "联邦名单更新失败");
            self.notify(&format!(
                "联邦名单更新失败：{error}\n下次配对变化或核心重启时会自动重试。"
            ))
            .await;
        }
    }

    async fn apply_inner(&self, names: &BTreeSet<String>) -> anyhow::Result<()> {
        let hosts_path = self.install_dir.join("fed/hosts");
        let on_disk = tokio::fs::read_to_string(&hosts_path)
            .await
            .unwrap_or_default();
        // 名单新增了名字：hosts 的新条目要 Tuwunel 重启才生效；只删名字 SIGUSR1 就够。
        let added = names
            .iter()
            .any(|name| !names_in_hosts(&on_disk, self.core_addr).contains(name));
        // SIGUSR1 无条件发：热加载是幂等的，也补上上次核心死在“写完文件、信号没发出去”之间的可能。
        self.write_in_place(
            &self.install_dir.join("fed/config.toml"),
            &allowlist_toml(names),
        )
        .await?;
        self.write_in_place(&hosts_path, &hosts(names, self.core_addr))
            .await?;
        let container = self.tuwunel_container().await?;
        if added {
            // 升级后首次启动可能在迁移数据库，不能被 10 秒后强杀（compose.yaml 里的宽限期同此）。
            self.engine.restart_container(&container, 300).await?;
            tracing::info!(%container, "联邦名单加了新服务器，已重启 Tuwunel 让 hosts 生效");
        } else {
            self.engine.kill_container(&container, "SIGUSR1").await?;
            tracing::info!(%container, "联邦名单已重写并热加载");
        }
        Ok(())
    }

    /// 原地截断写：不换 inode（单文件绑定挂载钉住挂载时的 inode，换名字容器读不到新内容）。
    async fn write_in_place(&self, path: &Path, content: &str) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .await
            .with_context(|| format!("打开 {}", path.display()))?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(())
    }

    /// Tuwunel 的容器名：核心自己的 Compose 项目里服务名为 `tuwunel` 的那个。
    async fn tuwunel_container(&self) -> anyhow::Result<String> {
        const PROJECT: &str = "com.docker.compose.project";
        let container = self
            .core_container
            .clone()
            .ok_or_else(|| anyhow::anyhow!("不知道核心自己的容器名"))?;
        let info = self
            .engine
            .inspect_container(&container)
            .await?
            .ok_or_else(|| anyhow::anyhow!("容器 {container} 不存在"))?;
        let project = info
            .config
            .and_then(|config| config.labels)
            .and_then(|labels| labels.get(PROJECT).cloned())
            .filter(|project| !project.is_empty())
            .ok_or_else(|| anyhow::anyhow!("容器 {container} 没有 Compose 项目标签"))?;
        let LabeledResources { containers, .. } = self
            .engine
            .labeled(PROJECT, &project, |labels| {
                labels
                    .get(SERVICE_LABEL)
                    .is_some_and(|service| service == "tuwunel")
            })
            .await?;
        match containers.as_slice() {
            [name] => Ok(name.clone()),
            other => Err(anyhow::anyhow!(
                "Compose 项目 {project} 里应有 1 个 tuwunel 容器，找到 {} 个：{other:?}",
                other.len()
            )),
        }
    }

    async fn notify(&self, body: &str) {
        let Some(notice) = &self.notice else { return };
        let content = serde_json::json!({"msgtype": "m.text", "body": body});
        if let Err(error) = notice.send(&content).await {
            tracing::error!(%error, "联邦名单的失败通知发不出去");
        }
    }
}
