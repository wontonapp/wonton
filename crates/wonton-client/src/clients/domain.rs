//! “HTTPS 和域名”设置页（ADR-0089 第 2–4 节）：后缀、服务商、凭据、订户协议和证书状态。
//!
//! 表单里输入的凭据由壳当场带走，ViewModel 只给变量名和“存过没有”，取回状态时不带任何凭据原文。

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::watch::Sender as WatchSender;
use wonton_protocol::{
    CertificateState, ClientsAction, ClientsDomainCertificate, ClientsDomainProvider,
    ClientsDomainState, ClientsDomainViewModel, ClientsFailure, ClientsProblem, CoreError,
    CredentialEntry, DnsProviderPreset, DomainSettings, ProxySettings, PutDomainRequest,
    SecretString,
};

use super::api;
use crate::home::HomeLink;
use crate::l10n::text;
use crate::scope::reuse;

/// 表单现在选中的服务商；由 `SelectDomainProvider` 改，取回设置时按已存的值重算。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    /// 预设的服务商代码。
    Preset(String),
    /// “其他”：lego 代码和 `键=值` 凭据由壳另外填。
    Other,
}

pub(crate) struct ClientsDomainLink {
    handle: Handle,
    home: Arc<HomeLink>,
    view: WatchSender<ClientsDomainViewModel>,
    /// 请求带的 `Accept-Language`，壳用 `SetLanguages` 告诉核心。
    languages: Mutex<Vec<String>>,
    /// 本地代理的凭据和端口随每次建连换；同一个代理上复用连接。
    client: Mutex<Option<(ProxySettings, reqwest::Client)>>,
    /// 同一时刻只跑一次请求；保存成功后的刷新排在正在跑的后面。
    queue: tokio::sync::Mutex<()>,
    /// 服务器上现在存着的服务商代码：重试时沿用，“其他”时也有。
    saved_provider: Mutex<Option<String>>,
    /// 取到过一份设置：换了语言要重新取，“详情”是核心按 App 的语言写的。
    fetched: AtomicBool,
    /// 上一次取回的服务商和域名（服务器上现在存的值）；表单与它不同就还在草稿里。
    served: Mutex<Option<(Option<String>, Selection)>>,
}

/// 设置页可以依赖这一项，空 ViewModel 的 `Loading` 够用；页面开着时第一次取设置由 `RefreshDomain` 发起。
impl ClientsDomainLink {
    pub(crate) fn new(
        handle: Handle,
        home: Arc<HomeLink>,
        view: Option<WatchSender<ClientsDomainViewModel>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            home,
            view: reuse(view, ClientsDomainViewModel::default()),
            languages: Mutex::new(Vec::new()),
            client: Mutex::new(None),
            queue: tokio::sync::Mutex::new(()),
            saved_provider: Mutex::new(None),
            fetched: AtomicBool::new(false),
            served: Mutex::new(None),
        })
    }

    pub(crate) fn sender(&self) -> WatchSender<ClientsDomainViewModel> {
        self.view.clone()
    }

    /// 壳告诉核心 App 的语言；取到过设置就重取，证书和 CAA 的说明跟着换语言。
    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        {
            let mut current = lock(&self.languages);
            if *current == languages {
                return;
            }
            *current = languages;
        }
        if self.fetched.load(Ordering::SeqCst) {
            self.refresh();
        }
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: ClientsAction) -> Result<(), CoreError> {
        match action {
            ClientsAction::RefreshDomain => {
                self.refresh();
                Ok(())
            }
            ClientsAction::CancelDomain => {
                self.cancel();
                Ok(())
            }
            ClientsAction::SelectDomainProvider { code } => {
                self.select(code);
                Ok(())
            }
            ClientsAction::SaveDomain {
                suffix,
                provider,
                credentials,
                other_credentials,
                accept_terms,
            } => self.save(
                suffix,
                provider,
                credentials,
                other_credentials,
                accept_terms,
            ),
            ClientsAction::RetryCertificate => {
                self.retry();
                Ok(())
            }
            ClientsAction::DismissDomainProblem => {
                self.view.send_modify(|view| view.problem = None);
                Ok(())
            }
            _ => Err(CoreError::NotReady {
                reason: "clients module is not wired yet".into(),
            }),
        }
    }

    /// 表单现在等于服务器上存的值吗（干净）；空选或两边都对不上算脏。
    fn form_matches(&self, served_suffix: &Option<String>, served_selection: &Selection) -> bool {
        let view = self.view.borrow();
        let selection = match (&view.selected_provider, view.other_provider) {
            (Some(code), false) => Selection::Preset(code.clone()),
            (None, true) => Selection::Other,
            _ => return false,
        };
        view.suffix == *served_suffix && selection == *served_selection
    }

    /// 离开页面：丢掉没保存的表单，回到服务器上现在存的值；证书、CAA 和保存进度不动。
    fn cancel(&self) {
        let Some((suffix, selection)) = lock(&self.served).clone() else {
            return;
        };
        let (selected_provider, other_provider, desec) = match &selection {
            Selection::Preset(code) => (Some(code.clone()), false, code == "desec"),
            Selection::Other => (None, true, false),
        };
        let credential_vars = {
            let view = self.view.borrow();
            match &selection {
                Selection::Preset(code) => view
                    .providers
                    .iter()
                    .find(|provider| provider.code.as_ref() == Some(code))
                    .map(|provider| provider.credential_vars.clone())
                    .unwrap_or_default(),
                Selection::Other => Vec::new(),
            }
        };
        self.view.send_modify(|view| {
            view.suffix = suffix;
            view.selected_provider = selected_provider;
            view.other_provider = other_provider;
            view.credential_vars = credential_vars;
            view.desec = desec;
            view.problem = None;
        });
    }

    fn refresh(self: &Arc<Self>) {
        let link = self.clone();
        self.spawn(async move { link.fetch().await });
    }

    async fn fetch(self: Arc<Self>) {
        let _guard = self.queue.lock().await;
        // 刚配对完、隧道还在握手的几秒里打开页面，第一次取多半只拿到“连不上”。
        // 取设置是只读的，这里自己等一等，不让页面停在错误上要人点重试。
        let mut attempts = 0;
        let outcome = loop {
            let outcome = match self.api() {
                Ok(api) => {
                    // 隧道刚起来时请求会挂着不回：自己限时，别让页面永远停在“还在加载”。
                    let fetch = async {
                        let (providers, domain) = tokio::join!(api.providers(), api.domain());
                        match (providers, domain) {
                            (Ok(providers), Ok(domain)) => Ok((providers, domain)),
                            (Err(failure), _) | (_, Err(failure)) => Err(failure),
                        }
                    };
                    match tokio::time::timeout(std::time::Duration::from_secs(15), fetch).await {
                        Ok(outcome) => outcome,
                        Err(_) => Err(api::Failure::transport("timeout".into())),
                    }
                }
                Err(failure) => Err(failure),
            };
            match outcome {
                Err(failure)
                    if attempts < 8
                        && matches!(
                            failure.problem.kind,
                            ClientsFailure::Other
                                | ClientsFailure::CredentialsFailed
                                | ClientsFailure::PluginUnavailable
                        ) =>
                {
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                outcome => break outcome,
            }
        };
        match outcome {
            Ok((presets, domain)) => {
                self.fetched.store(true, Ordering::SeqCst);
                let selection = resolve(&presets, domain.provider.as_deref());
                let served_suffix = domain.suffix.clone();
                // 第一次取（还没有服务端值）总是采用服务器的；之后表单脏了才保留。
                let dirty =
                    lock(&self.served).is_some() && !self.form_matches(&served_suffix, &selection);
                *lock(&self.saved_provider) = domain.provider.clone();
                *lock(&self.served) = Some((served_suffix, selection.clone()));
                let mut view = build(domain, presets, &selection);
                // 正在填的表单不被刷新冲掉；干净的表单换语言或换设备时整页跟着服务器走。
                if dirty {
                    let current = self.view.borrow();
                    view.suffix = current.suffix.clone();
                    view.selected_provider = current.selected_provider.clone();
                    view.other_provider = current.other_provider;
                    view.credential_vars = current.credential_vars.clone();
                    view.desec = current.desec;
                }
                self.view.send_modify(|current| *current = view);
            }
            Err(failure) => self.fail_load(failure),
        }
    }

    /// 只改本机表单，不写服务器。
    fn select(&self, code: Option<String>) {
        let selection = match code {
            Some(code) => Selection::Preset(code),
            None => Selection::Other,
        };
        let (credential_vars, desec, selected_provider, other_provider) = {
            let view = self.view.borrow();
            let credential_vars = match &selection {
                Selection::Preset(code) => view
                    .providers
                    .iter()
                    .find(|provider| provider.code.as_ref() == Some(code))
                    .map(|provider| provider.credential_vars.clone())
                    .unwrap_or_default(),
                Selection::Other => Vec::new(),
            };
            let desec = matches!(&selection, Selection::Preset(code) if code == "desec");
            let (selected_provider, other_provider) = match &selection {
                Selection::Preset(code) => (Some(code.clone()), false),
                Selection::Other => (None, true),
            };
            (credential_vars, desec, selected_provider, other_provider)
        };
        self.view.send_modify(|view| {
            view.selected_provider = selected_provider;
            view.other_provider = other_provider;
            view.credential_vars = credential_vars;
            view.desec = desec;
            view.problem = None;
        });
    }

    /// 校验通过才发 PUT；失败不动表单。
    fn save(
        self: &Arc<Self>,
        suffix: Option<String>,
        provider: Option<String>,
        credentials: Vec<CredentialEntry>,
        other_credentials: String,
        accept_terms: bool,
    ) -> Result<(), CoreError> {
        if !accept_terms {
            return Err(invalid(text("clients-domain-terms", &[])));
        }
        let (editable, credentials_set, current_provider, other) = {
            let view = self.view.borrow();
            (
                view.suffix_editable,
                view.credentials_set,
                view.selected_provider.clone(),
                view.other_provider,
            )
        };
        let suffix = if editable {
            let suffix = normalize_suffix(&suffix.unwrap_or_default());
            if suffix.is_empty() {
                return Err(invalid(text("clients-domain-suffix", &[])));
            }
            match wonton_protocol::suffix_problem(&suffix) {
                None => {}
                Some(wonton_protocol::SuffixProblem::Invalid) => {
                    return Err(invalid(text("clients-domain-suffix-invalid", &[])));
                }
                Some(wonton_protocol::SuffixProblem::Ip) => {
                    return Err(invalid(text("clients-domain-suffix-ip", &[])));
                }
            }
            Some(suffix)
        } else {
            None
        };
        let (provider, credentials) = if other {
            let code = provider.unwrap_or_default().trim().to_owned();
            if code.is_empty() {
                return Err(invalid(text("clients-domain-provider", &[])));
            }
            (code, parse_other(&other_credentials)?)
        } else {
            let code = provider.or(current_provider).unwrap_or_default();
            if code.is_empty() {
                return Err(invalid(text("clients-domain-provider", &[])));
            }
            let changed = lock(&self.saved_provider).as_deref() != Some(code.as_str());
            if credentials.is_empty() && (!credentials_set || changed) {
                return Err(invalid(text("clients-domain-credentials", &[])));
            }
            (code, credentials)
        };
        let request = PutDomainRequest {
            suffix,
            provider,
            credentials,
            accept_terms,
        };
        self.view.send_modify(|view| {
            view.saving = true;
            view.problem = None;
        });
        self.put(request);
        Ok(())
    }

    /// 续期失败或过期后重试：沿用服务器上现在存的后缀、服务商和凭据，不动表单。
    fn retry(self: &Arc<Self>) {
        let terms_accepted = self.view.borrow().terms_accepted;
        let provider = lock(&self.saved_provider)
            .clone()
            .filter(|provider| !provider.is_empty());
        let Some(provider) = provider else {
            self.view.send_modify(|view| {
                view.problem = Some(ClientsProblem {
                    kind: ClientsFailure::InvalidInput,
                    message: text("clients-domain-provider", &[]),
                });
            });
            return;
        };
        self.view.send_modify(|view| {
            view.saving = true;
            view.problem = None;
        });
        self.put(PutDomainRequest {
            suffix: None,
            provider,
            credentials: Vec::new(),
            accept_terms: terms_accepted,
        });
    }

    fn put(self: &Arc<Self>, request: PutDomainRequest) {
        let link = self.clone();
        self.spawn(async move {
            let outcome = match link.api() {
                Ok(api) => api.save(&request).await,
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(domain) => {
                    *lock(&link.saved_provider) = Some(request.provider);
                    link.view.send_modify(|view| {
                        view.saving = false;
                        view.problem = None;
                        view.state = ClientsDomainState::Ready;
                        apply(view, domain);
                    });
                }
                Err(failure) => link.fail_action(failure),
            }
        });
    }

    /// 首次取设置失败：整页进失败态，壳给一句话和重试（重试就是 `RefreshDomain`）。
    fn fail_load(&self, failure: api::Failure) {
        tracing::debug!(
            kind = failure.problem.kind.as_str(),
            "clients domain load failed"
        );
        let problem = failure.problem;
        self.view.send_modify(|view| {
            view.saving = false;
            view.problem = Some(problem.clone());
            view.state = ClientsDomainState::Failed { problem };
        });
    }

    /// 保存/重试被拒：页面和表单还在，只提示这一条。
    fn fail_action(&self, failure: api::Failure) {
        tracing::debug!(
            kind = failure.problem.kind.as_str(),
            "clients domain operation failed"
        );
        self.view.send_modify(|view| {
            view.saving = false;
            view.problem = Some(failure.problem);
        });
    }

    fn api(&self) -> Result<api::Api, api::Failure> {
        // 已知隧道不通就立刻失败，不等超时。
        if let Some(reason) = self.home.tunnel_outage() {
            return Err(api::Failure::transport(reason));
        }
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let base = self.home.core_url().ok_or_else(not_connected)?;
        let client = {
            let mut cached = lock(&self.client);
            match cached.as_ref() {
                Some((settings, client)) if *settings == proxy => client.clone(),
                _ => {
                    let client = api::proxied_client(&proxy)?;
                    *cached = Some((proxy.clone(), client.clone()));
                    client
                }
            }
        };
        Ok(api::Api::new(base, client, lock(&self.languages).clone()))
    }

    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.handle.spawn(task);
    }
}

/// 已存的服务商命中预设就选它，否则算“其他”；没存过时选排第一的预设（没有预设就选“其他”）。
fn resolve(presets: &[DnsProviderPreset], stored: Option<&str>) -> Selection {
    if let Some(code) = stored {
        return if presets.iter().any(|preset| preset.code == code) {
            Selection::Preset(code.to_owned())
        } else {
            Selection::Other
        };
    }
    match presets.first() {
        Some(preset) => Selection::Preset(preset.code.clone()),
        None => Selection::Other,
    }
}

fn build(
    domain: DomainSettings,
    presets: Vec<DnsProviderPreset>,
    selection: &Selection,
) -> ClientsDomainViewModel {
    let providers = presets
        .into_iter()
        .map(|preset| ClientsDomainProvider {
            code: Some(preset.code),
            name: preset.name,
            recommended: preset.recommended,
            signup_url: preset.signup_url,
            credential_vars: preset.credential_vars,
        })
        .chain(std::iter::once(ClientsDomainProvider {
            code: None,
            name: String::new(),
            recommended: false,
            signup_url: None,
            credential_vars: Vec::new(),
        }))
        .collect::<Vec<_>>();
    let (selected_provider, other_provider) = match selection {
        Selection::Preset(code) => (Some(code.clone()), false),
        Selection::Other => (None, true),
    };
    let credential_vars = match selection {
        Selection::Preset(code) => providers
            .iter()
            .find(|provider| provider.code.as_ref() == Some(code))
            .map(|provider| provider.credential_vars.clone())
            .unwrap_or_default(),
        Selection::Other => Vec::new(),
    };
    let desec = matches!(selection, Selection::Preset(code) if code == "desec");
    let mut view = ClientsDomainViewModel {
        state: ClientsDomainState::Ready,
        providers,
        selected_provider,
        other_provider,
        credential_vars,
        desec,
        saving: false,
        problem: None,
        ..ClientsDomainViewModel::default()
    };
    apply(&mut view, domain);
    view
}

/// 把服务器给的当前状态写进 ViewModel；选中的服务商由调用方维持。
fn apply(view: &mut ClientsDomainViewModel, domain: DomainSettings) {
    let (certificate, retryable, details) = match domain.certificate {
        CertificateState::Renewing {
            not_after_ms,
            error,
        } => (
            CertificateState::Renewing {
                not_after_ms,
                error: error.clone(),
            },
            true,
            Some(error),
        ),
        CertificateState::Expired {
            not_after_ms,
            error,
        } => (
            CertificateState::Expired {
                not_after_ms,
                error: error.clone(),
            },
            true,
            Some(error),
        ),
        other => (other, false, None),
    };
    let suffix = domain.suffix;
    view.suffix = suffix;
    view.suffix_editable = domain.suffix_editable;
    view.credentials_set = domain.credentials_set;
    view.terms_accepted = domain.terms_accepted;
    view.certificate = ClientsDomainCertificate {
        state: certificate,
        retryable,
        details,
    };
    view.caa = domain.caa;
    view.caa_record = domain.caa_record;
    view.lan_record = domain.lan_record;
    view.lan_target = domain.lan_target;
    view.lan_record_name = view.suffix.as_ref().map(|suffix| format!("*.lan.{suffix}"));
}

/// 域名去掉两边空白、末尾的点和大小写差异；其余校验交给 core。
fn normalize_suffix(raw: &str) -> String {
    raw.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// “其他”的 `键=值` 原文：一行一项，键和值都去掉两边空白；写错的行报第几行。
fn parse_other(raw: &str) -> Result<Vec<CredentialEntry>, CoreError> {
    let mut entries = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line_error = || {
            invalid(text(
                "clients-domain-other-line",
                &[("line", (index + 1).to_string().into())],
            ))
        };
        let (name, value) = line.split_once('=').ok_or_else(line_error)?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return Err(line_error());
        }
        entries.push(CredentialEntry {
            name: name.to_owned(),
            value: SecretString::new(value),
        });
    }
    if entries.is_empty() {
        return Err(invalid(text("clients-domain-other-empty", &[])));
    }
    Ok(entries)
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidInput { reason }
}

fn not_connected() -> api::Failure {
    api::Failure::not_connected()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
