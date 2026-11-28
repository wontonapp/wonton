//! 签发和续期：排队、准入、失败记录、发布后的权限与唤醒、通知。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, ensure};

use super::acme::{DNS_ENV_FILE, dotenv_value};
use super::settings::invalid;
use super::view::now_ms;
use super::{Domain, Store};
use crate::entry_settings::EntrySettings;

impl Domain {
    /// 后缀或服务商换了：账号还是那个，但持久化的身份、CSR 和准备状态属于旧名字，删掉重建。
    /// 旧的 `site.pem` 留着，新证书发布前入口照常用它。
    pub(super) fn reset_prepared(&self) {
        for name in ["prepared.json", "identity.json", "requests/site.csr"] {
            let path = self.install_dir.join("entry").join(name);
            if let Err(error) = std::fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::error!(path = %path.display(), %error, "清掉旧的证书准备状态失败");
            }
        }
    }

    /// 准入条件够了就排一次签发；续期循环和保存都走这里。
    pub(super) fn spawn_issue(self: &Arc<Self>, force: bool) {
        let store = match self.effective() {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!(%error, "域名设置读不了，这一轮跳过续期");
                let text = format!("{error:#}");
                let this = self.clone();
                tokio::spawn(async move {
                    this.notify("entry-domain-unreadable", &text).await;
                });
                return;
            }
        };
        if !self.ready(&store) {
            return;
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(self.clone().run_issue(store, force));
    }

    /// 续期循环的一轮：条件不够就如实跳过，够了就排一次签发。
    pub fn renew_if_configured(self: &Arc<Self>) {
        self.spawn_issue(false);
    }

    fn ready(&self, store: &Store) -> bool {
        if !store.terms_accepted || store.suffix.is_none() {
            return false;
        }
        let settings = EntrySettings::load(&self.install_dir).ok().flatten();
        if settings
            .as_ref()
            .is_some_and(|settings| settings.external_certificate)
        {
            return false;
        }
        let registration = settings.and_then(|settings| settings.registration);
        registration.is_some() || (store.provider.is_some() && store.has_credentials())
    }

    pub(super) async fn run_issue(self: Arc<Self>, snapshot: Store, force: bool) {
        let _serial = self.issuance.lock().await;
        // 排队时设置可能已经变过：拿到锁后以现状为准，快照过期就跳过——变更方自己排的
        // 任务会按新设置签。旧任务若按旧快照重新 prepare，会把新设置的准备状态和证书
        // 都写成旧的，续期从此对不上。
        match self.effective() {
            Ok(current) if current.clone().issuance_snapshot() == snapshot.issuance_snapshot() => {
                if !force && self.fresh(&current) {
                    tracing::info!("证书还新且上次没失败，跳过重复的签发");
                } else {
                    self.issue_locked(&current).await;
                }
            }
            Ok(_) => {
                tracing::info!("签发任务排队期间设置已变更，跳过这次（按新设置另排的任务负责）");
            }
            Err(error) => {
                tracing::error!(%error, "重读域名设置失败，跳过排队中的签发");
            }
        }
        self.pending.fetch_sub(1, Ordering::SeqCst);
    }

    /// 已经发布给现在这个名字的证书还新（到期超过 30 天）、上一次没失败：
    /// 重复点保存或另一台设备再保存一次不必重新签一张。
    ///
    /// 不能拿 [`Self::certificate_state`] 判：它在 `pending > 0` 时先给 `Issuing`，
    /// 而 `run_issue` 一定是在排过队之后跑的。换过后缀时 `reset_prepared` 删掉了身份，
    /// 旧证书虽然新也不是给这个名字的，所以要按叶证书的 SAN 再看一眼。
    pub(super) fn fresh(&self, store: &Store) -> bool {
        let Some(suffix) = store.suffix.as_deref() else {
            return false;
        };
        if store.last_error.is_some() {
            return false;
        }
        let entry = self.install_dir.join("entry");
        // 准备状态属于这次的服务商和名字：换过就删了，删了就得重签。
        if !entry.join("prepared.json").is_file() {
            return false;
        }
        let certs = entry.join("certs");
        if !certs.join("site.key").is_file() {
            return false;
        }
        let Some(not_after_ms) = self.not_after_ms() else {
            return false;
        };
        if not_after_ms <= now_ms() + 30 * 24 * 60 * 60 * 1000 {
            return false;
        }
        crate::entry_certificate::published_names(&certs.join("site.pem"))
            .iter()
            .any(|name| name == suffix)
    }

    /// 持有签发锁时真正跑一轮签发；结果进设置文件，失败原证书保留。
    async fn issue_locked(&self, store: &Store) {
        let result = self.issue_once(store).await;
        let error = result.as_ref().err().map(|error| format!("{error:#}"));
        if let Err(write) = self.update(|state| state.last_error = error.clone()).await {
            tracing::error!(%write, "记录证书签发结果失败");
            return;
        }
        match result {
            Ok(()) => {
                if let Err(error) = self.refresh_entry().await {
                    tracing::error!(%error, "入口证书更新没写出去");
                    self.notify("entry-certificate-reload-failed", &format!("{error:#}"))
                        .await;
                }
            }
            Err(error) => {
                tracing::error!(%error, "入口证书签发失败，保留原证书");
                self.notify("entry-certificate-renew-failed", &format!("{error:#}"))
                    .await;
            }
        }
    }

    /// 证书发布了：先把新证书的权限交给入口用户，再叫醒续期循环。那轮对账先按新证书写配置、
    /// 再发 USR2；反过来的顺序会让入口拿旧名字的 crt-list 去读新证书。
    async fn refresh_entry(&self) -> Result<()> {
        crate::entry_compose::prepare_permissions(&self.install_dir)?;
        self.notify_refresh();
        Ok(())
    }

    /// 叫醒续期循环立刻对一次账（新证书已发布）。
    pub(crate) fn notify_refresh(&self) {
        self.refresh.notify_one();
    }

    /// 等到一张新证书发布；续期循环收到就立刻对一次账。
    pub async fn refreshed(&self) {
        self.refresh.notified().await;
    }

    /// 装机路径：同步签发一次，失败让安装停下。主人自己放 `site.pem` 的装机只核对文件在不在。
    pub async fn issue_now(self: &Arc<Self>) -> Result<()> {
        let _serial = self.issuance.lock().await;
        if EntrySettings::load(&self.install_dir)
            .map_err(invalid)?
            .is_some_and(|settings| settings.external_certificate)
        {
            ensure!(
                self.install_dir.join("entry/certs/site.pem").is_file(),
                "自己的证书缺少 entry/certs/site.pem"
            );
            return Ok(());
        }
        let store = self.effective()?;
        ensure!(store.terms_accepted, "没有勾选订户协议");
        ensure!(store.suffix.is_some(), "没有配置后缀");
        let result = self.issue_once(&store).await;
        let error = result.as_ref().err().map(|error| format!("{error:#}"));
        self.update(|state| state.last_error = error).await?;
        result
    }

    async fn issue_once(&self, store: &Store) -> Result<()> {
        let suffix = store.suffix.as_deref().context("没有配置后缀")?;
        let acme = self.acme()?;
        let manager = self.manager(store)?;
        let settings = EntrySettings::load(&self.install_dir)?;
        let registration = settings
            .as_ref()
            .and_then(|settings| settings.registration.as_ref());
        if !store.credentials.is_empty() {
            self.write_dns_env(&store.credentials)?;
        }
        let outcome = async {
            let account = manager
                .prepare(
                    suffix,
                    registration.map(|registration| &registration.acme_dns),
                    registration.map(|registration| registration.acme_dns_url.as_str()),
                )
                .await?;
            if let (Some(settings), Some(registration)) = (settings.as_ref(), registration)
                && let Some(service) = settings.service(&self.data_dir)?
            {
                service.bind_caa(&registration.home, &account).await?;
                let identity =
                    crate::entry_dns::caa_identity(&acme.directory_url).with_context(|| {
                        format!(
                            "认不出 {} 的 CAA 身份，不能核对 CAA 绑定",
                            acme.directory_url
                        )
                    })?;
                crate::entry_dns::verify_caa(suffix, &account, &identity, &settings.acme_resolvers)
                    .await?;
            }
            manager.issue(suffix).await
        }
        .await;
        let _ = std::fs::remove_file(self.install_dir.join(DNS_ENV_FILE));
        let _ = std::fs::remove_file(self.install_dir.join("entry/acme/dns.env"));
        outcome
    }

    fn write_dns_env(&self, credentials: &BTreeMap<String, String>) -> Result<()> {
        let path = self.install_dir.join(DNS_ENV_FILE);
        let directory = path.parent().context("凭据文件没有父目录")?;
        std::fs::create_dir_all(directory)?;
        std::fs::set_permissions(
            directory,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )?;
        let mut text = String::new();
        for (name, value) in credentials {
            text.push_str(name);
            text.push('=');
            text.push_str(&dotenv_value(value));
            text.push('\n');
        }
        crate::install::write_private(&path, &text)
    }

    async fn notify(&self, id: &str, error: &str) {
        let Some(notices) = &self.notices else {
            return;
        };
        let text = crate::l10n::text(&(notices.languages)(), id, &[("error", error.into())]);
        if let Err(error) = notices
            .send(&serde_json::json!({"msgtype":"m.text","body":text}))
            .await
        {
            tracing::error!(%error, "入口状态通知失败");
        }
    }
}
