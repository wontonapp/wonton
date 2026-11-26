//! 联邦配对代理范围、私用名的允许名单和整机黑名单一起生成、热加载。
//!
//! 单文件 bind mount 钉住 inode：原地写完并落盘后才让 Tuwunel 重读（管理房间命令，不经 Docker 的信号）。
//! 有名字的服务器只把配对目标交给 CONNECT 代理，其余由 Tuwunel 直接发现和连接。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use tokio::sync::watch;

use super::{Matrix, NoticeRoom};

#[cfg(test)]
mod admin_reload_tests;
mod blocks;
pub use blocks::FederationBlocks;
mod routing;
pub use routing::{config_toml, is_private};
#[cfg(test)]
mod failure_tests;
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

/// 盯着配对名单，变了就把 Tuwunel 的配置跟上来；顺便按待钉窗口开关 `allow_invalid_tls_certificates`。
pub struct FederationConfig {
    /// 发给 Tuwunel 的重读命令走它。
    matrix: Arc<Matrix>,
    /// 安装目录，`secrets/tuwunel/wonton-fed.toml` 由这里重写。
    install_dir: PathBuf,
    /// 数据目录，`core/peers.json` 从这里读（待钉窗口的依据）。
    data_dir: PathBuf,
    notice: Option<Arc<NoticeRoom>>,
    server_name: String,
    /// 上一次成功让 Tuwunel 用上的 `allow_invalid` 值；写成功但热加载失败时留空重试。
    window_applied: Mutex<Option<bool>>,
}

impl FederationConfig {
    /// 宿主上直接跑核心时（没有安装目录或 Matrix）什么都不做：联邦只在正式安装里生效。
    pub fn spawn(
        matrix: Option<Arc<Matrix>>,
        install_dir: Option<PathBuf>,
        data_dir: PathBuf,
        notice: Option<Arc<NoticeRoom>>,
        mut peers: watch::Receiver<BTreeSet<String>>,
        server_name: String,
        blocks: Arc<FederationBlocks>,
    ) {
        let (Some(matrix), Some(install_dir)) = (matrix, install_dir) else {
            tracing::debug!("没有安装目录或 Matrix，联邦名单交给 Tuwunel 的安装配置");
            return;
        };
        let config = Arc::new(Self {
            matrix,
            install_dir,
            data_dir,
            notice,
            server_name,
            window_applied: Mutex::new(None),
        });
        tokio::spawn(async move {
            let mut forbidden = blocks.subscribe();
            let mut refresh = tokio::time::interval(std::time::Duration::from_secs(2));
            let initial = peers.borrow_and_update().clone();
            let initial_forbidden = forbidden.borrow_and_update().clone();
            config.apply(initial, initial_forbidden).await;
            config.apply_window().await;
            loop {
                tokio::select! {
                    result = peers.changed() => if result.is_err() { break; },
                    result = forbidden.changed() => if result.is_err() { break; },
                    _ = refresh.tick() => {
                        if let Err(error) = blocks.reload() {
                            tracing::error!(%error, "联邦黑名单读取失败，保留上一次已生效的名单");
                        }
                        config.apply_window().await;
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
        if let Err(failure) = self.apply_inner(&names, &forbidden).await {
            tracing::error!(error = %failure.error, kind = ?failure.kind, "联邦名单更新失败");
            self.notify(failure.kind).await;
        }
    }

    /// 按待钉窗口（ADR-0112 §5b）开关 Tuwunel 的 `allow_invalid_tls_certificates`：还有没钉住的老对端就
    /// 开着；钉齐或窗口到期就关掉。值没变不动文件、不热加载；读不懂记录时保留现值（放宽着也不断线），下一
    /// 轮再看。
    async fn apply_window(&self) {
        let state = match crate::peer_window::window_state(
            Some(&self.data_dir),
            crate::build_info::version(),
        ) {
            Ok(state) => state,
            Err(error) => {
                tracing::error!(%error, "读不出待钉窗口状态，allow_invalid 保持原值");
                return;
            }
        };
        let needed = state.needed;
        if *self.window_applied.lock().expect("窗口状态锁中毒") == Some(needed) {
            return;
        }
        if state.expired > 0 {
            tracing::warn!(
                expired = state.expired,
                "待钉窗口已到期，对端还没升级；握手仍按证书名字放行"
            );
        }
        let path = self.install_dir.join("secrets/tuwunel/tuwunel.toml");
        if let Err(error) = crate::peer_window::set_allow_invalid(&path, needed).await {
            tracing::error!(%error, "改写 allow_invalid_tls_certificates 失败");
            return;
        }
        if let Err(error) = self.matrix.reload_config().await {
            tracing::error!(%error, "改了 allow_invalid 之后热加载失败，下一轮重试");
            return;
        }
        *self.window_applied.lock().expect("窗口状态锁中毒") = Some(needed);
        tracing::info!(
            allow_invalid = needed,
            "按待钉窗口改写了 Tuwunel 的证书校验"
        );
        if !needed {
            self.notify_window_closed().await;
        }
    }

    /// 窗口关掉时给主人一句话（对端还没升级、钉住了都会关）。
    async fn notify_window_closed(&self) {
        let Some(notice) = &self.notice else { return };
        let body = crate::l10n::text(&(notice.languages)(), "federation-window-closed", &[]);
        let content = serde_json::json!({"msgtype": "m.text", "body": body});
        if let Err(error) = notice.send(&content).await {
            tracing::error!(%error, "窗口关闭的通知发不出去");
        }
    }

    async fn apply_inner(
        &self,
        names: &BTreeSet<String>,
        forbidden: &BTreeSet<String>,
    ) -> Result<(), Failure> {
        for name in names.iter().chain(forbidden) {
            routing::validate_name(name).map_err(Failure::of(FailureKind::InvalidName))?;
        }
        write_in_place(
            &self.install_dir.join(FILE),
            &config_toml(&self.server_name, names, forbidden),
        )
        .await
        .map_err(Failure::of(FailureKind::Write))?;
        // 重读无条件发：热加载是幂等的，也补上上次核心死在“写完文件、命令没发出去”之间的可能。
        self.matrix
            .reload_config()
            .await
            .map_err(|error| Failure::of(FailureKind::Reload)(error.into()))?;
        tracing::info!("联邦名单已重写并热加载");
        Ok(())
    }

    /// 主人只看到类别；路径和原始错误在核心日志里（AGENTS.md“界面设计”，ADR-0043）。
    async fn notify(&self, kind: FailureKind) {
        let Some(notice) = &self.notice else { return };
        let body = crate::l10n::text(&(notice.languages)(), kind.message_id(), &[]);
        let content = serde_json::json!({"msgtype": "m.text", "body": body});
        if let Err(error) = notice.send(&content).await {
            tracing::error!(%error, "联邦名单的失败通知发不出去");
        }
    }
}

/// 名单文件，相对安装目录。
const FILE: &str = "secrets/tuwunel/wonton-fed.toml";

/// 更新失败的类别，决定通知房间里给主人的那句话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    /// 名单里有不合法的服务器名（黑名单文件是主人能编辑的）。
    InvalidName,
    /// 名单文件写不进去：权限、磁盘，或安装目录不完整。
    Write,
    /// 文件写好了，但找不到 Tuwunel 容器或 Docker 不让发热加载信号。
    Reload,
}

impl FailureKind {
    fn message_id(self) -> &'static str {
        match self {
            Self::InvalidName => "federation-update-invalid-name",
            Self::Write => "federation-update-write-failed",
            Self::Reload => "federation-update-reload-failed",
        }
    }
}

#[derive(Debug)]
struct Failure {
    kind: FailureKind,
    error: anyhow::Error,
}

impl Failure {
    fn of(kind: FailureKind) -> impl FnOnce(anyhow::Error) -> Self {
        move |error| Self { kind, error }
    }
}

/// 原地截断写：不换 inode（单文件绑定挂载钉住挂载时的 inode，换名字容器读不到新内容）。
///
/// 文件不在就新建（权限同安装时的 0600）：这份文件是核心写给 Tuwunel 的输出，不是输入，缺了照名单写出来
/// 就是对的。目录不在不建：`secrets/tuwunel/` 由安装创建、整目录挂给 Tuwunel，它不在说明安装不完整，如实报错。
async fn write_in_place(path: &Path, content: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .await
        .with_context(|| format!("打开 {}", path.display()))?;
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}
