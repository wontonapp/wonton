//! 联邦配对代理范围、私用名的允许名单和整机黑名单一起生成、热加载。
//!
//! 单文件 bind mount 钉住 inode：原地写完并落盘后才向 Tuwunel 发 SIGUSR1。
//! 有名字的服务器只把配对目标交给 CONNECT 代理，其余由 Tuwunel 直接发现和连接。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::watch;

use super::NoticeRoom;
use crate::plugin::docker::{Engine, LabeledResources};

mod blocks;
pub use blocks::FederationBlocks;
mod routing;
pub use routing::{config_toml, is_private};
mod files;
#[cfg(test)]
mod proxy_tests;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod tests;

pub(crate) use files::{allowlist_toml, blocks_toml};

/// 核心给 Tuwunel 开的联邦出站代理：核心在内部网络上的别名和端口。`deploy/install/compose.yaml` 里核心的
/// 网络别名和 `WONTON_FEDERATION_LISTEN` 用的是同一个值，一起改。
pub(crate) const PROXY_ADDR: &str = "federation-proxy.wonton.internal:8448";

/// Compose 项目里 Tuwunel 容器的服务名标签。
const SERVICE_LABEL: &str = "com.docker.compose.service";

/// 盯着配对名单，变了就把 Tuwunel 的配置跟上来。
pub struct FederationConfig {
    engine: Engine,
    /// 安装目录，`secrets/tuwunel/wonton-fed.toml` 由这里重写。
    install_dir: PathBuf,
    /// 核心自己的容器名，用它找同一个 Compose 项目里的 Tuwunel。
    core_container: Option<String>,
    notice: Option<Arc<NoticeRoom>>,
    server_name: String,
}

impl FederationConfig {
    /// 宿主上直接跑核心时（没有安装目录或容器名）什么都不做：联邦只在正式安装里生效。
    pub fn spawn(
        engine: Engine,
        install_dir: Option<PathBuf>,
        core_container: Option<String>,
        notice: Option<Arc<NoticeRoom>>,
        mut peers: watch::Receiver<BTreeSet<String>>,
        server_name: String,
        blocks: Arc<FederationBlocks>,
    ) {
        let (Some(install_dir), Some(core_container)) = (install_dir, core_container) else {
            tracing::debug!("没有安装目录或核心容器名，联邦名单交给 Tuwunel 的安装配置");
            return;
        };
        let config = Arc::new(Self {
            engine,
            install_dir,
            core_container: Some(core_container),
            notice,
            server_name,
        });
        tokio::spawn(async move {
            let mut forbidden = blocks.subscribe();
            let mut refresh = tokio::time::interval(std::time::Duration::from_secs(2));
            let initial = peers.borrow_and_update().clone();
            let initial_forbidden = forbidden.borrow_and_update().clone();
            config.apply(initial, initial_forbidden).await;
            loop {
                tokio::select! {
                    result = peers.changed() => if result.is_err() { break; },
                    result = forbidden.changed() => if result.is_err() { break; },
                    _ = refresh.tick() => {
                        if let Err(error) = blocks.reload() {
                            tracing::error!(%error, "联邦黑名单读取失败，保留上一次已生效的名单");
                        }
                        continue;
                    }
                }
                let names = peers.borrow_and_update().clone();
                let denied = forbidden.borrow_and_update().clone();
                config.apply(names, denied).await;
            }
            tracing::warn!("联邦配置来源已关闭，配置停在最后的值");
        });
    }

    async fn apply(&self, names: BTreeSet<String>, forbidden: BTreeSet<String>) {
        if let Err(error) = self.apply_inner(&names, &forbidden).await {
            tracing::error!(%error, "联邦名单更新失败");
            self.notify(&format!("{error:#}")).await;
        }
    }

    async fn apply_inner(
        &self,
        names: &BTreeSet<String>,
        forbidden: &BTreeSet<String>,
    ) -> anyhow::Result<()> {
        for name in names.iter().chain(forbidden) {
            routing::validate_name(name)?;
        }
        self.write_in_place(
            &self.install_dir.join("secrets/tuwunel/wonton-fed.toml"),
            &config_toml(&self.server_name, names, forbidden),
        )
        .await?;
        // SIGUSR1 无条件发：热加载是幂等的，也补上上次核心死在“写完文件、信号没发出去”之间的可能。
        let container = self.tuwunel_container().await?;
        self.engine.kill_container(&container, "SIGUSR1").await?;
        tracing::info!(%container, "联邦名单已重写并热加载");
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

    /// 失败通知发进“Wonton”私聊：第一段是一句人话（AGENTS.md 的界面设计），宿主路径和错误原文只进
    /// 详情段，正文里认不出它们。
    async fn notify(&self, error: &str) {
        let Some(notice) = &self.notice else { return };
        let langs = (notice.languages)();
        let body = format!(
            "{}\n\n{}",
            crate::l10n::text(&langs, "fed-update-failed", &[]),
            crate::l10n::text(
                &langs,
                "fed-update-failed-detail",
                &[("error", error.into())],
            ),
        );
        let content = serde_json::json!({"msgtype": "m.text", "body": body});
        if let Err(error) = notice.send(&content).await {
            tracing::error!(%error, "联邦名单的失败通知发不出去");
        }
    }
}
