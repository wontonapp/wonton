//! 核心生成 Mailpit 的私有配置；目录绑定挂载允许原子替换配置文件。

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, lchown};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::{Mutex, broadcast};
use wonton_protocol::MemberStatus;

use crate::config::PLUGIN_SMTP_HOST;
use crate::matrix::NoticeRoom;
use crate::members::Members;
use crate::plugin::docker::{Engine, LabeledResources};
use crate::plugin::hostdir::{Owner, ensure_owned_dir};

pub struct Component {
    engine: Engine,
    core_container: String,
    config_dir: PathBuf,
    owner: String,
    members: Arc<Members>,
    password: String,
    files_owner: Owner,
    restart_pending: Mutex<bool>,
}

impl Component {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        engine: Engine,
        core_container: String,
        state_dir: &Path,
        owner: &str,
        members: Arc<Members>,
        uid: u32,
        gid: u32,
    ) -> anyhow::Result<Arc<Self>> {
        let dir = state_dir.join("mailpit");
        fs::create_dir_all(state_dir)?;
        let core_owner = fs::metadata(state_dir)?;
        ensure_owned_dir(
            &dir,
            Owner {
                uid: core_owner.uid(),
                gid: core_owner.gid(),
            },
        )?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let files_owner = Owner { uid, gid };
        let config_dir = dir.join("config");
        ensure_owned_dir(&config_dir, files_owner)?;
        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700))?;
        tls_cert(&config_dir, files_owner)?;
        ensure_owned_dir(&dir.join("data"), files_owner)?;
        let password = crate::secret::load_or_create(&dir.join("api-password"))?;
        atomic_write(
            &config_dir.join("auth.htpasswd"),
            &format!("core:{password}\n"),
            files_owner,
        )?;
        let runtime = config_dir.join("runtime.env");
        // 已存在的配置可能是上次写完、尚未重启就退出的；核心每次启动都补一次重启。
        let restart_pending = runtime.try_exists()?;
        atomic_write(&runtime, &runtime_env(owner, &members), files_owner)?;
        Ok(Arc::new(Self {
            engine,
            core_container,
            config_dir,
            owner: owner.to_owned(),
            members,
            password,
            files_owner,
            restart_pending: Mutex::new(restart_pending),
        }))
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    /// 只认核心所在的 Compose 项目，不能误接同一台 Docker 上另一套核心的 Mailpit。
    pub async fn container(&self) -> anyhow::Result<String> {
        self.optional_container()
            .await?
            .context("Compose 项目里没有 Mailpit 容器")
    }

    pub(crate) async fn optional_container(&self) -> anyhow::Result<Option<String>> {
        const PROJECT: &str = "com.docker.compose.project";
        let info = self
            .engine
            .inspect_container(&self.core_container)
            .await?
            .ok_or_else(|| anyhow::anyhow!("核心容器不存在：{}", self.core_container))?;
        let project = info
            .config
            .and_then(|config| config.labels)
            .and_then(|labels| labels.get(PROJECT).cloned())
            .filter(|project| !project.is_empty())
            .ok_or_else(|| anyhow::anyhow!("核心容器没有 Compose 项目标签"))?;
        let LabeledResources { containers, .. } = self
            .engine
            .labeled(PROJECT, &project, |labels| {
                labels
                    .get("com.docker.compose.service")
                    .is_some_and(|service| service == "mailpit")
                    && labels
                        .get("com.docker.compose.oneoff")
                        .is_none_or(|oneoff| !oneoff.eq_ignore_ascii_case("true"))
            })
            .await?;
        match containers.as_slice() {
            [] => Ok(None),
            [name] => Ok(Some(name.clone())),
            other => anyhow::bail!(
                "Compose 项目 {project} 应至多有 1 个 Mailpit 容器，找到 {} 个",
                other.len()
            ),
        }
    }

    pub async fn refresh_recipients(&self) -> anyhow::Result<()> {
        let mut pending = self.restart_pending.lock().await;
        let runtime = self.config_dir.join("runtime.env");
        let content = runtime_env(&self.owner, &self.members);
        if fs::read_to_string(&runtime).with_context(|| format!("读取 {}", runtime.display()))?
            != content
        {
            *pending = true;
            atomic_write(&runtime, &content, self.files_owner)?;
        }
        if *pending {
            let container = self.container().await?;
            self.engine.stop_container(&container).await?;
            self.engine.start_container(&container).await?;
            *pending = false;
        }
        Ok(())
    }

    pub fn spawn_members(
        self: &Arc<Self>,
        notices: Option<Arc<NoticeRoom>>,
        languages: Arc<dyn Fn() -> crate::plugin::languages::Languages + Send + Sync>,
    ) {
        // 先订阅再做首次刷新，不能漏掉启动期间的加入/移除。
        let mut events = self.members.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = this.refresh_recipients().await {
                    tracing::error!(%error, "Mailpit 收件人名单更新失败");
                    if let Some(notices) = &notices {
                        let body = crate::l10n::text(
                            &languages(),
                            "chat-mail-recipients-failed",
                            &[("error", error.to_string().as_str().into())],
                        );
                        if let Err(error) = notices
                            .send(&serde_json::json!({"msgtype": "m.text", "body": body}))
                            .await
                        {
                            tracing::error!(%error, "Mailpit 失败通知发送失败");
                        }
                    }
                }
                match events.recv().await {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }
}

fn runtime_env(owner: &str, members: &Members) -> String {
    let names: BTreeSet<String> = std::iter::once(owner.to_owned())
        .chain(
            members
                .list()
                .into_iter()
                .filter(|member| member.status == MemberStatus::Joined)
                .map(|member| member.username),
        )
        .collect();
    // 用户名的协议字符集不含 regex/shell 元字符；仍逐字节转义，避免损坏的持久化记录扩大放行范围。
    let escaped: Vec<String> = names
        .iter()
        .map(|name| {
            name.chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                        ch.to_string()
                    } else {
                        format!("\\x{{{:x}}}", u32::from(ch))
                    }
                })
                .collect()
        })
        .collect();
    format!(
        "export MP_SMTP_ALLOWED_RECIPIENTS='^({})@wonton\\.internal$'\n",
        escaped.join("|")
    )
}

/// 内部 SMTP 的 STARTTLS 证书：自签，只服务插件网络里的别名，成对的留着不换（同 Tuwunel 那份）。
fn tls_cert(config_dir: &Path, owner: Owner) -> anyhow::Result<()> {
    let cert = config_dir.join("smtp-tls.crt");
    let key = config_dir.join("smtp-tls.key");
    if cert.exists() && key.exists() {
        return Ok(());
    }
    let pair = rcgen::generate_simple_self_signed(vec![PLUGIN_SMTP_HOST.to_owned()])
        .map_err(|e| anyhow::anyhow!("生成 {PLUGIN_SMTP_HOST} 的自签证书失败：{e}"))?;
    atomic_write(&cert, &pair.cert.pem(), owner)?;
    atomic_write(&key, &pair.signing_key.serialize_pem(), owner)?;
    Ok(())
}

fn atomic_write(path: &Path, content: &str, owner: Owner) -> anyhow::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", crate::secret::random_token()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        let meta = file.metadata()?;
        if meta.uid() != owner.uid || meta.gid() != owner.gid {
            lchown(&temporary, Some(owner.uid), Some(owner.gid))?;
        }
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(path.parent().context("配置没有父目录")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("写入 Mailpit 配置 {}", path.display()))
}

#[cfg(test)]
#[path = "component_tests.rs"]
mod tests;
