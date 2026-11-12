//! 自己的域名和它的证书（ADR-0089）：设置落盘、运行时签发、CAA 状态。
//!
//! 设置文件是 `entry/domain.json`（0600）：后缀、服务商、只写不读的凭据、订户协议的勾选和最近一次
//! 签发失败的错误。装机时写下的环境变量（`entry/settings.json` 和 `WONTON_ACME_*`）是初值，有设置
//! 文件以后以设置文件为准。“不要名字”的安装没有 `entry/settings.json`，也能只靠环境给 ACME 坐标。
//!
//! 凭据只在签发时写成 lego 的 dotenv（0600），只挂进 lego 容器，用完删掉。

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use wonton_protocol::{
    CaaState, CertificateState, CredentialEntry, DomainSettings, LanRecordState, PutDomainRequest,
};

use crate::entry_certificate::{CertificateConfig, CertificateManager, validate_domain};
use crate::entry_settings::{EntrySettings, parse_resolvers};
use crate::matrix::NoticeRoom;
use crate::plugin::docker::Engine;
use crate::plugin::names::{Instance, LABEL_COMPONENT, LABEL_MANAGED_BY};

const DOMAIN_FILE: &str = "entry/domain.json";
const DNS_ENV_FILE: &str = "entry/secrets/dns.env";
pub const SUFFIX_CHANGED_HEADER: &str = "X-Wonton-Clients-Notice";
pub const PREVIOUS_SUFFIX_HEADER: &str = "X-Wonton-Previous-Suffix";

/// 设置文件的内容。凭据的值原样存在这里，接口只回有没有。
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    credentials: BTreeMap<String, String>,
    /// 装机给的 dotenv 文件：值不解析、原样交给 lego；App 提交了新凭据后作废。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    env_file: Option<std::path::PathBuf>,
    #[serde(default)]
    terms_accepted: bool,
    /// 最近一次签发失败的原因；成功后清空。
    #[serde(default)]
    last_error: Option<String>,
}

impl Store {
    /// 有凭据可用：App 提交过，或装机给了原样 dotenv 文件。
    fn has_credentials(&self) -> bool {
        !self.credentials.is_empty() || self.env_file.is_some()
    }

    /// 签发快照：只留影响签发的设置。`last_error` 是签发的输出，算进相等会把
    /// “上一任务收尾写结果”误判成“设置变了”，误杀排队的下一次——修正凭据后
    /// 得白等一个续期周期。
    fn issuance_snapshot(mut self) -> Store {
        self.last_error = None;
        self
    }
}

/// 设置项的失败，接口按类别回 400 或 409。
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
}

/// 一次保存的结果：后缀变了要给壳一个“第三方 App 要重填”的标记。
pub struct Saved {
    pub previous_suffix: Option<String>,
    pub suffix_changed: bool,
    pub provider_changed: bool,
}

/// 域名设置和签发。API 和续期循环共用一份。
pub struct Domain {
    install_dir: PathBuf,
    data_dir: PathBuf,
    engine: Engine,
    core_container: String,
    /// 核心的实例名：入口容器按组件标签找时用它限定自己这套，别热加载别家的入口。
    instance: Instance,
    notices: Option<Arc<NoticeRoom>>,
    /// 串行化签发；同时来两次就排队，后一次用最新的设置。
    issuance: Mutex<()>,
    /// 已经排队的签发数；大于零时证书状态是 `issuing`。
    pending: AtomicUsize,
    /// 设置文件的读写锁，签发不占着它。
    writes: Mutex<()>,
    /// CAA 结果的短缓存（后缀加账号为键，60 秒）：设置页刷新别每次都等 DNS。
    caa_cache: Mutex<Option<(std::time::Instant, String, CaaState)>>,
}

struct Acme {
    directory_url: String,
    ca_file: Option<PathBuf>,
    resolvers: Vec<String>,
    propagation_disable_ans: bool,
    email: Option<String>,
    network: String,
}

impl Domain {
    pub fn new(
        install_dir: PathBuf,
        data_dir: PathBuf,
        engine: Engine,
        core_container: String,
        instance: Instance,
        notices: Option<Arc<NoticeRoom>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            install_dir,
            data_dir,
            engine,
            core_container,
            instance,
            notices,
            issuance: Mutex::new(()),
            pending: AtomicUsize::new(0),
            writes: Mutex::new(()),
            caa_cache: Mutex::new(None),
        })
    }

    /// 装机时定的服务器名（`entry/settings.json` 的 `domain`）。有它后缀就不能改。
    pub fn fixed_suffix(&self) -> Result<Option<String>> {
        Ok(EntrySettings::load(&self.install_dir)?.map(|settings| settings.domain))
    }

    /// 现在的设置：有设置文件用文件，否则用装机环境的初值。
    fn effective(&self) -> Result<Store> {
        let mut store = match std::fs::read(self.install_dir.join(DOMAIN_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("读不了域名设置")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(error) => return Err(error).context("读不了域名设置"),
        };
        if !self.install_dir.join(DOMAIN_FILE).exists()
            && let Some(settings) = EntrySettings::load(&self.install_dir)?
        {
            store.suffix = Some(settings.domain);
            store.provider = settings.dns_provider;
            store.terms_accepted = settings.acme_accept_terms;
            if let Some(path) = settings.dns_env_file {
                dotenv_names(&path)?;
                store.env_file = Some(path);
            }
        }
        Ok(store)
    }

    /// lego 用的 ACME 坐标：设置文件优先，没有就用环境（“不要名字”的安装没有设置文件）。
    fn acme(&self) -> Result<Acme> {
        if let Some(settings) = EntrySettings::load(&self.install_dir)? {
            return Ok(Acme {
                directory_url: settings.acme_directory.context("没有配置 ACME directory")?,
                ca_file: settings.acme_ca,
                resolvers: settings.acme_resolvers,
                propagation_disable_ans: settings.acme_propagation_disable_ans,
                email: settings.acme_email,
                network: settings.acme_network,
            });
        }
        let setting = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Ok(Acme {
            directory_url: setting("WONTON_ACME_DIRECTORY").context("没有配置 ACME directory")?,
            ca_file: setting("WONTON_ACME_CA_FILE").map(PathBuf::from),
            resolvers: match setting("WONTON_ACME_RESOLVERS") {
                Some(raw) => parse_resolvers(&raw)?,
                None => Vec::new(),
            },
            propagation_disable_ans: setting("WONTON_ACME_PROPAGATION_DISABLE_ANS").as_deref()
                == Some("true"),
            email: setting("WONTON_ACME_EMAIL"),
            network: setting("WONTON_ACME_NETWORK").unwrap_or_else(|| "bridge".into()),
        })
    }

    fn manager(&self, store: &Store) -> Result<CertificateManager> {
        let acme = self.acme()?;
        let settings = EntrySettings::load(&self.install_dir)?;
        let registration = settings
            .as_ref()
            .and_then(|settings| settings.registration.as_ref());
        // 有家名服务时走 acme-dns；自己的服务商只在没有家名服务时装进 lego。
        let custom = registration.is_none();
        let credentials_set = store.has_credentials();
        CertificateManager::new(
            self.install_dir.clone(),
            self.engine.clone(),
            CertificateConfig {
                directory_url: acme.directory_url,
                ca_file: acme.ca_file,
                dns_resolvers: acme.resolvers,
                dns_propagation_disable_ans: acme.propagation_disable_ans,
                email: acme.email,
                accept_terms: store.terms_accepted,
                network: acme.network,
                host_install_dir: self.install_dir.clone(),
                include_lan: true,
                lan_acme_dns: registration.map(|registration| registration.lan_acme_dns.clone()),
                dns_provider: (custom && credentials_set)
                    .then(|| store.provider.clone())
                    .flatten(),
                // 装机给的文件原样交给 lego；App 提交的凭据才由我们写成 dns.env。
                dns_env_file: (custom && credentials_set).then(|| {
                    store
                        .env_file
                        .clone()
                        .unwrap_or_else(|| self.install_dir.join(DNS_ENV_FILE))
                }),
            },
        )
    }

    /// 在设置文件的锁里读-改-写：并发请求看到的是同一个当前值。
    async fn apply<T>(
        &self,
        change: impl FnOnce(&mut Store) -> Result<T, DomainError>,
    ) -> Result<T, DomainError> {
        let _writing = self.writes.lock().await;
        let mut store = self.effective().map_err(invalid)?;
        let out = change(&mut store)?;
        let directory = self.install_dir.join("entry/secrets");
        std::fs::create_dir_all(&directory).map_err(|error| invalid(error.into()))?;
        std::fs::set_permissions(
            &directory,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .map_err(|error| invalid(error.into()))?;
        crate::install::write_private(
            &self.install_dir.join(DOMAIN_FILE),
            &serde_json::to_string_pretty(&store).map_err(|error| invalid(error.into()))?,
        )
        .map_err(invalid)?;
        Ok(out)
    }

    async fn update(&self, change: impl FnOnce(&mut Store)) -> Result<()> {
        self.apply(|store| {
            change(store);
            Ok(())
        })
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// 设置页看到的整份状态。
    pub async fn view(&self) -> Result<DomainSettings> {
        let store = self.effective()?;
        let fixed = self.fixed_suffix()?;
        Ok(DomainSettings {
            suffix: fixed.clone().or(store.suffix.clone()),
            suffix_editable: fixed.is_none(),
            provider: store.provider.clone(),
            credentials_set: store.has_credentials(),
            terms_accepted: store.terms_accepted,
            certificate: self.certificate_state(&store),
            caa: self.caa_state(&store).await,
            lan_record: LanRecordState::Unknown,
        })
    }

    /// 保存设置并排一次签发。`suffix` 在服务器名定死时只能是同一个值，改不了。
    /// 凭据为空时沿用已存的；家名服务的安装本来就不用服务商和凭据，两者也都可空。
    pub async fn save(self: &Arc<Self>, request: PutDomainRequest) -> Result<Saved, DomainError> {
        if !request.accept_terms {
            return Err(DomainError::Invalid(
                "先勾选 Let's Encrypt 的订户协议".into(),
            ));
        }
        let settings = EntrySettings::load(&self.install_dir).map_err(invalid)?;
        // 主人自己放了 site.pem 的安装不跑 ACME：收下设置也没人签，直接说清楚。
        if settings
            .as_ref()
            .is_some_and(|settings| settings.external_certificate)
        {
            return Err(DomainError::Conflict(
                "装的是自己的证书（WONTON_ENTRY_CERTIFICATE=external），核心不签发".into(),
            ));
        }
        let registered = settings
            .as_ref()
            .is_some_and(|settings| settings.registration.is_some());
        if !(registered && request.provider.is_empty()) {
            validate_provider(&request.provider).map_err(invalid)?;
        }
        validate_credentials(&request.credentials).map_err(invalid)?;
        let fixed = self.fixed_suffix().map_err(invalid)?;
        let provider = request.provider;
        let credentials: std::collections::BTreeMap<String, String> = request
            .credentials
            .into_iter()
            .map(|entry| (entry.name, entry.value.expose().to_owned()))
            .collect();
        let saved = self
            .apply(|store| {
                let previous = store.suffix.clone();
                let suffix = match &fixed {
                    Some(fixed) => {
                        if request
                            .suffix
                            .as_deref()
                            .is_some_and(|value| value != fixed)
                        {
                            return Err(DomainError::Conflict(
                                "服务器名已经定了，后缀只能换服务商和凭据".into(),
                            ));
                        }
                        fixed.clone()
                    }
                    None => {
                        let suffix = request
                            .suffix
                            .clone()
                            .or_else(|| previous.clone())
                            .ok_or_else(|| DomainError::Invalid("先填一个后缀".into()))?;
                        validate_domain(&suffix).map_err(invalid)?;
                        suffix
                    }
                };
                let changed = previous.as_deref().is_some_and(|old| old != suffix);
                // 空服务商＝沿用现状（家名安装的常态），不算变更，不然每次保存都会误重置签发准备。
                let effective_provider = if provider.is_empty() {
                    store.provider.clone()
                } else {
                    Some(provider.clone())
                };
                let provider_changed = store.provider != effective_provider;
                // 首次配置或换服务商必须一起提交凭据，否则设置收下了却永远不签，也没人知道为什么。
                if credentials.is_empty()
                    && !(registered && provider.is_empty())
                    && (provider_changed || !store.has_credentials())
                {
                    return Err(DomainError::Invalid(
                        "首次配置或换服务商时要一起提交这个服务商的凭据".into(),
                    ));
                }
                store.suffix = Some(suffix);
                if let Some(provider) = effective_provider {
                    store.provider = Some(provider);
                }
                if !credentials.is_empty() {
                    store.credentials = credentials.clone();
                    // App 提交的凭据取代装机给的文件，别再让旧文件抢着生效。
                    store.env_file = None;
                }
                store.terms_accepted = true;
                Ok(Saved {
                    previous_suffix: changed.then_some(previous).flatten(),
                    suffix_changed: changed,
                    provider_changed,
                })
            })
            .await?;
        if saved.suffix_changed || saved.provider_changed {
            // 删准备状态要和进行中的签发互斥：签发正用着它们。
            let _serial = self.issuance.lock().await;
            self.reset_prepared();
        }
        self.spawn_issue();
        Ok(saved)
    }

    /// 后缀或服务商换了：账号还是那个，但持久化的身份、CSR 和准备状态属于旧名字，删掉重建。
    /// 旧的 `site.pem` 留着，新证书发布前入口照常用它。
    fn reset_prepared(&self) {
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
    fn spawn_issue(self: &Arc<Self>) {
        let Ok(store) = self.effective() else { return };
        if !self.ready(&store) {
            return;
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(self.clone().run_issue(store));
    }

    /// 续期循环的一轮：条件不够就如实跳过，够了就排一次签发。
    pub fn renew_if_configured(self: &Arc<Self>) {
        self.spawn_issue();
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

    async fn run_issue(self: Arc<Self>, snapshot: Store) {
        let _serial = self.issuance.lock().await;
        // 排队时设置可能已经变过：拿到锁后以现状为准，快照过期就跳过——变更方自己排的
        // 任务会按新设置签。旧任务若按旧快照重新 prepare，会把新设置的准备状态和证书
        // 都写成旧的，续期从此对不上。
        match self.effective() {
            Ok(current) if current.clone().issuance_snapshot() == snapshot.issuance_snapshot() => {
                self.issue_locked(&current).await
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
                let loaded = async {
                    crate::entry_compose::prepare_permissions(&self.install_dir)?;
                    self.reload_entry().await
                }
                .await;
                if let Err(error) = loaded {
                    tracing::error!(%error, "入口证书热加载失败");
                    self.notify("entry-certificate-reload-failed", &error.to_string())
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

    /// 入口容器在就让它重读证书，不在就只发布。
    async fn reload_entry(&self) -> Result<()> {
        // 入口容器由核心按组件标签建（C5）：`wonton.core.component=entry` + 本实例，不再是 compose 服务。
        let found = self
            .engine
            .labeled(LABEL_MANAGED_BY, "core", |labels| {
                self.instance.owns(labels)
                    && labels.get(LABEL_COMPONENT).map(String::as_str) == Some("entry")
            })
            .await
            .context("找入口容器")?;
        let Some(container) = found.containers.first() else {
            return Ok(());
        };
        self.engine
            .kill_container(container, "SIGUSR2")
            .await
            .map_err(Into::into)
    }

    fn certificate_state(&self, store: &Store) -> CertificateState {
        if self.pending.load(Ordering::SeqCst) > 0 {
            return CertificateState::Issuing;
        }
        let now = now_ms();
        match self.not_after_ms() {
            Some(not_after) if not_after > now => match &store.last_error {
                Some(error) => CertificateState::Renewing {
                    not_after_ms: not_after,
                    error: error.clone(),
                },
                None => CertificateState::Valid {
                    not_after_ms: not_after,
                },
            },
            Some(not_after) => CertificateState::Expired {
                not_after_ms: not_after,
                error: store
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "证书已过期".into()),
            },
            None => match &store.last_error {
                Some(error) => CertificateState::Expired {
                    not_after_ms: 0,
                    error: error.clone(),
                },
                None => CertificateState::None,
            },
        }
    }

    /// 已发布证书的到期时间（Unix 毫秒）。取第一张不是 CA 的叶证书：自己拼的链条可能把 CA 放前面。
    fn not_after_ms(&self) -> Option<u64> {
        let pem = std::fs::read(self.install_dir.join("entry/certs/site.pem")).ok()?;
        let mut fallback = None;
        for der in rustls_pemfile::certs(&mut Cursor::new(pem)).flatten() {
            let Ok((_, certificate)) = x509_parser::parse_x509_certificate(&der) else {
                continue;
            };
            let seconds = u64::try_from(certificate.validity().not_after.timestamp()).ok()?;
            let millis = seconds.checked_mul(1000);
            let ca = certificate
                .basic_constraints()
                .ok()
                .flatten()
                .is_some_and(|constraints| constraints.value.ca);
            if ca {
                fallback.get_or_insert(millis);
            } else {
                return millis;
            }
        }
        fallback.flatten()
    }

    async fn caa_state(&self, store: &Store) -> CaaState {
        let Some(suffix) = store.suffix.as_deref() else {
            return CaaState::Unknown;
        };
        let Ok(acme) = self.acme() else {
            return CaaState::Unknown;
        };
        // 认不出当前 CA 的身份就不猜：显示 Unknown，签发路径会当场拒绝。
        let Some(identity) = crate::entry_dns::caa_identity(&acme.directory_url) else {
            return CaaState::Unknown;
        };
        let Ok(manager) = self.manager(store) else {
            return CaaState::Unknown;
        };
        let Ok(account) = manager.account_uri() else {
            return CaaState::Unknown;
        };
        let key = format!("{suffix}|{account}|{identity}");
        if let Some((at, cached_key, state)) = &*self.caa_cache.lock().await
            && cached_key == &key
            && at.elapsed() < std::time::Duration::from_secs(60)
        {
            return *state;
        }
        let Ok(records) = crate::entry_dns::caa_issues(suffix, &acme.resolvers).await else {
            return CaaState::Unknown;
        };
        let state = crate::entry_dns::caa_state(&records, &account, &identity);
        *self.caa_cache.lock().await = Some((std::time::Instant::now(), key, state));
        state
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

fn invalid(error: anyhow::Error) -> DomainError {
    DomainError::Invalid(format!("{error:#}"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// lego 的 provider 代码只许小写字母和数字（和 `CertificateManager` 同一套规则）。
fn validate_provider(provider: &str) -> Result<()> {
    if provider.is_empty()
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        bail!("服务商代码只能是小写字母和数字");
    }
    Ok(())
}

/// 凭据变量名按环境变量，值不能带换行（否则一个值能伪造出第二行）。
fn validate_credentials(credentials: &[CredentialEntry]) -> Result<()> {
    if credentials.is_empty() {
        return Ok(());
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in credentials {
        ensure!(
            !entry.name.is_empty()
                && entry
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
            "凭据名 {} 不是环境变量名",
            entry.name
        );
        ensure!(seen.insert(&entry.name), "凭据名 {} 重复", entry.name);
        ensure!(
            !entry.value.expose().contains(['\n', '\r']),
            "凭据 {} 的值不能有换行",
            entry.name
        );
        // lego 用 godotenv 读 env 文件，单引号里是字面量：值里带单引号表示不了，直接拒绝。
        ensure!(
            !entry.value.expose().contains('\''),
            "凭据 {} 的值不能有单引号",
            entry.name
        );
    }
    Ok(())
}

/// 值里有 godotenv 有意义的字符时用单引号包起来。godotenv 的单引号是字面量（不解 `\`、不展开 `$`），
/// 值里只要没有单引号（校验时已拒绝）就原样写。未引用值里 godotenv 会展开 `$VAR`、吃掉 `\$` 前的
/// 反斜杠，所以 `$` 和 `\` 都不能走未引用路径（实测 godotenv v1.5.1，lego v5.5.2 引的就是它）。
pub(super) fn dotenv_value(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:+".contains(&byte));
    if plain {
        return value.to_owned();
    }
    format!("'{value}'")
}

/// 装机给的 dotenv 原样交给 lego，值不解析（godotenv v1.5.1 的双引号、转义和展开语义归它，
/// 我们不复刻）。这里只认变量名做存在性检查：`export ` 前缀可带空白，键在第一个 `=` 左边，
/// `#` 开头是注释，没有键的文件报错。
pub(super) fn dotenv_names(path: &Path) -> Result<std::collections::BTreeSet<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("读 {}", path.display()))?;
    let mut names = std::collections::BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = match line.strip_prefix("export") {
            Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
            _ => line,
        };
        let Some((name, _)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() {
            names.insert(name.to_owned());
        }
    }
    ensure!(!names.is_empty(), "{} 里没有可用的凭据变量", path.display());
    Ok(names)
}

#[cfg(test)]
#[path = "domain_tests.rs"]
mod domain_tests;
