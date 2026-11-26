//! 中继表设置页（ADR-0048）：经本地代理读写家里服务器的中继表。
//!
//! 表和档位由核心管。App 只在核心最近给的那一份上改：加、改、删、挪档、n0 兜底都先改本机这份并把
//! `dirty` 标起来，「存下来」才把整张表经 `PUT /_wonton/api/relays` 交回去，核心合并后回一整份新的替换掉本机这份；
//! 「丢弃」直接回到核心最近给的那一份。请求走本地代理，核心凭隧道身份认出这台已配对设备，和配对码一样。
//!
//! 表由 `WONTON_RELAYS` 固定时整张只读；某一条由核心标成不能改时，改它当场拒绝，不等到保存。
//! 保存期间进一步的改动和保存都按「正在保存」拒掉，因为保存交出去的是发起那一刻的整张表。

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, ProxySettings, RelaySettingEntry, RelaySettingUpdate, RelaySettingsAction,
    RelaySettingsPhase, RelaySettingsSnapshot, RelaySettingsUpdate, RelaySettingsView, RelaySource,
    RelayState, SecretString,
};

use super::HomeLink;
use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};
use wonton_tunnel::relays::relay_from_item;

/// 固定的管理接口路径，拼在连接信息给的核心地址上。
const PATH: &str = "/_wonton/api/relays";
/// 整个请求（连上、发出、读完回应）最多等这么久：设置页的读写不该一直转圈。
const TIMEOUT: Duration = Duration::from_secs(30);

/// 本机对中继表的最近一份核心快照，和作废在途回应的代数。
///
/// 每次请求和本地编辑都加一代：迟到的读或保存回来时，哦对不上就丢掉，不盖掉更新的状态。
#[derive(Default)]
pub(super) struct RelaysState {
    snapshot: Option<RelaySettingsSnapshot>,
    epoch: u64,
}

impl HomeLink {
    /// `HomeAction::RelaySettings` 的入口：读、改、存、丢都在这里，壳只发操作。
    pub(crate) fn relay_settings(
        self: &Arc<Self>,
        action: RelaySettingsAction,
    ) -> Result<(), CoreError> {
        match action {
            RelaySettingsAction::Refresh => {
                self.refresh_relays();
                Ok(())
            }
            RelaySettingsAction::Add {
                url,
                token,
                quic_port,
                tier,
            } => self.edit_relays(|view| add(view, url, token, quic_port, tier)),
            RelaySettingsAction::Update {
                url,
                token,
                quic_port,
                tier,
            } => self.edit_relays(|view| update(view, url, token, quic_port, tier)),
            RelaySettingsAction::Remove { url } => self.edit_relays(|view| remove(view, url)),
            RelaySettingsAction::Move { url, tier, before } => {
                self.edit_relays(|view| move_entry(view, url, tier, before))
            }
            RelaySettingsAction::SetN0 { on } => self.edit_relays(|view| set_n0(view, on)),
            RelaySettingsAction::Save => self.save_relays(),
            RelaySettingsAction::Discard => self.discard_relays(),
        }
    }

    /// 正在保存时读不进来：要交出去的那份表正在路上，替换掉本机这份只会让两边对不上。
    /// 还没配对、还没连上时把原因挂在 `load_problem` 上，不进后台任务。
    fn refresh_relays(self: &Arc<Self>) {
        if self.view().relays.phase == Some(RelaySettingsPhase::Saving) {
            return;
        }
        let target = match self.relay_target() {
            Ok(target) => target,
            Err(reason) => {
                self.view
                    .send_modify(|view| fail_load(&mut view.relays, reason));
                return;
            }
        };
        let epoch = {
            let mut state = self.lock_relays();
            state.epoch += 1;
            state.epoch
        };
        self.view.send_modify(|view| {
            view.relays.phase = Some(RelaySettingsPhase::Loading);
            view.relays.load_problem = None;
        });
        self.runtime.spawn(self.clone().load_relays(target, epoch));
    }

    async fn load_relays(self: Arc<Self>, target: Target, epoch: u64) {
        let result = call(Method::GET, target, None).await;
        let mut state = self.lock_relays();
        if state.epoch != epoch {
            return;
        }
        match result {
            Ok(snapshot) => {
                state.snapshot = Some(snapshot.clone());
                self.view
                    .send_modify(|view| apply_snapshot(&mut view.relays, &snapshot));
            }
            Err(reason) => self
                .view
                .send_modify(|view| fail_load(&mut view.relays, reason)),
        }
    }

    /// 在一份本地副本上改一步。改完置为可保存（`dirty`），清掉上一次保存的失败。
    ///
    /// 只有改成功才作废在途的读；改失败（比如地址不合法）时保留它，免得表卡在 `Loading`。
    fn edit_relays(
        self: &Arc<Self>,
        edit: impl FnOnce(&mut RelaySettingsView) -> Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        let mut state = self.lock_relays();
        if self.view().relays.phase == Some(RelaySettingsPhase::Saving) {
            return Err(not_ready("relays-busy"));
        }
        let mut outcome = Ok(());
        self.view.send_modify(|view| {
            outcome = edit(&mut view.relays);
            if outcome.is_ok() {
                let relays = &mut view.relays;
                relays.phase = Some(RelaySettingsPhase::Ready);
                relays.dirty = true;
                relays.load_problem = None;
                relays.save_problem = None;
            }
        });
        if outcome.is_ok() {
            state.epoch += 1;
        }
        outcome
    }

    /// 把整张表交给核心。交出去的是发起这一刻的那一份；回来之前不许再改、也不许再存。
    fn save_relays(self: &Arc<Self>) -> Result<(), CoreError> {
        {
            let view = self.view();
            // 还没从核心取到过表时保存会把服务端的表清空。
            if view.relays.phase.is_none() {
                return Err(not_ready("relays-not-loaded"));
            }
        }
        let target = self
            .relay_target()
            .map_err(|reason| CoreError::NotReady { reason })?;
        let (epoch, update) = {
            let mut state = self.lock_relays();
            let view = self.view();
            let relays = &view.relays;
            if relays.phase == Some(RelaySettingsPhase::Saving) {
                return Err(not_ready("relays-busy"));
            }
            if relays.locked_by_env {
                return Err(not_ready("relays-locked"));
            }
            // 一次都没成功取到过表时 entries 是空的，交上去等于把服务端由内置和签名列表来的项全删了。
            if state.snapshot.is_none() {
                return Err(not_ready("relays-not-loaded"));
            }
            state.epoch += 1;
            (state.epoch, update_from(relays))
        };
        self.view.send_modify(|view| {
            view.relays.phase = Some(RelaySettingsPhase::Saving);
            view.relays.save_problem = None;
        });
        self.runtime
            .spawn(self.clone().put_relays(target, update, epoch));
        Ok(())
    }

    async fn put_relays(self: Arc<Self>, target: Target, update: RelaySettingsUpdate, epoch: u64) {
        let result = call(Method::PUT, target, Some(&update)).await;
        let mut state = self.lock_relays();
        if state.epoch != epoch {
            return;
        }
        match result {
            Ok(snapshot) => {
                state.snapshot = Some(snapshot.clone());
                self.view
                    .send_modify(|view| apply_snapshot(&mut view.relays, &snapshot));
            }
            // 改动留着，主人修好原因可以直接再存。
            Err(reason) => self.view.send_modify(|view| {
                let relays = &mut view.relays;
                relays.phase = Some(RelaySettingsPhase::Failed {
                    reason: Some(reason.clone()),
                });
                relays.save_problem = Some(reason);
            }),
        }
    }

    /// 丢掉没保存的修改，回到核心最近给的那一份；一份都没拿到过就回到空表。
    fn discard_relays(self: &Arc<Self>) -> Result<(), CoreError> {
        if self.view().relays.phase == Some(RelaySettingsPhase::Saving) {
            return Err(not_ready("relays-busy"));
        }
        let mut state = self.lock_relays();
        state.epoch += 1;
        let snapshot = state.snapshot.clone();
        self.view.send_modify(|view| match snapshot {
            Some(snapshot) => apply_snapshot(&mut view.relays, &snapshot),
            None => view.relays = RelaySettingsView::default(),
        });
        Ok(())
    }

    pub(super) fn lock_relays(&self) -> std::sync::MutexGuard<'_, RelaysState> {
        self.relays
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 请求要用的核心地址和本地代理；缺一个都说明现在发不出去。
    fn relay_target(&self) -> Result<Target, String> {
        let base = self
            .core_url()
            .ok_or_else(|| l10n_text("relays-not-paired"))?;
        let proxy = self
            .view()
            .proxy
            .ok_or_else(|| l10n_text("relays-not-connected"))?;
        Ok(Target {
            url: base.join(PATH).expect("fixed relays path always joins"),
            proxy,
        })
    }
}

struct Target {
    url: Url,
    proxy: ProxySettings,
}

/// 发一次请求，读回核心给的那一份表。非 2xx 带上服务器自己的说明。
async fn call(
    method: Method,
    target: Target,
    body: Option<&RelaySettingsUpdate>,
) -> Result<RelaySettingsSnapshot, String> {
    let client = reqwest::Client::builder()
        // 不复用连接：本地代理到回环地址，建连几乎没有代价；空闲的连接可能刚好被对面关掉。
        .pool_max_idle_per_host(0)
        .proxy(
            reqwest::Proxy::all(proxy_url(&target.proxy)).map_err(|error| {
                l10n_text_with("relays-invalid-proxy", "reason", error.to_string())
            })?,
        )
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| l10n_text_with("relays-http-client", "reason", error.to_string()))?;
    let mut request = client.request(method, target.url);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|error| transport(&error))?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.map_err(|error| transport(&error))?;
    if !(200..300).contains(&status) {
        return Err(server_error(status, &bytes));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| l10n_text_with("relays-invalid-response", "reason", error.to_string()))
}

fn transport(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        l10n!(
            CATALOG,
            language(),
            "relays-timeout",
            seconds = TIMEOUT.as_secs()
        )
    } else {
        l10n!(
            CATALOG,
            language(),
            "relays-request-failed",
            reason = error.to_string()
        )
    }
}

/// 核心的错误响应：`{"error": {"kind": …, "message": …}}`。
#[derive(serde::Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(serde::Deserialize)]
struct ErrorDetail {
    kind: String,
    message: String,
}

fn server_error(status: u16, body: &[u8]) -> String {
    match serde_json::from_slice::<ErrorBody>(body) {
        Ok(body) => l10n!(
            CATALOG,
            language(),
            "relays-server-error",
            message = body.error.message,
            kind = body.error.kind
        ),
        Err(_) => l10n!(
            CATALOG,
            language(),
            "relays-server-status",
            status = u64::from(status)
        ),
    }
}

fn not_ready(key: &str) -> CoreError {
    CoreError::NotReady {
        reason: l10n_text(key),
    }
}

fn invalid(key: &str, value: impl Into<String>) -> CoreError {
    CoreError::InvalidInput {
        reason: l10n_text_with(key, "value", value.into()),
    }
}

fn l10n_text(key: &str) -> String {
    l10n!(CATALOG, language(), key)
}

fn l10n_text_with(key: &str, name: &str, value: String) -> String {
    match name {
        "reason" => l10n!(CATALOG, language(), key, reason = value),
        "value" => l10n!(CATALOG, language(), key, value = value),
        _ => unreachable!("relays strings use reason or value"),
    }
}

fn apply_snapshot(view: &mut RelaySettingsView, snapshot: &RelaySettingsSnapshot) {
    view.phase = Some(RelaySettingsPhase::Ready);
    view.entries = snapshot.entries.clone();
    view.n0 = snapshot.n0;
    view.locked_by_env = snapshot.locked_by_env;
    view.warnings = snapshot.warnings.clone();
    view.dirty = false;
    view.load_problem = None;
    view.save_problem = None;
}

fn fail_load(view: &mut RelaySettingsView, reason: String) {
    view.phase = Some(RelaySettingsPhase::Failed {
        reason: Some(reason.clone()),
    });
    view.load_problem = Some(reason);
}

fn update_from(view: &RelaySettingsView) -> RelaySettingsUpdate {
    RelaySettingsUpdate {
        n0: view.n0,
        entries: view
            .entries
            .iter()
            .map(|entry| RelaySettingUpdate {
                url: entry.url.clone(),
                token: entry.token.clone(),
                quic_port: entry.quic_port,
                tier: entry.tier,
            })
            .collect(),
    }
}

fn add(
    view: &mut RelaySettingsView,
    url: String,
    token: Option<SecretString>,
    quic_port: Option<u16>,
    tier: Option<u8>,
) -> Result<(), CoreError> {
    editable(view)?;
    let (url, fragment_port) = validate_url(&url)?;
    if view.entries.iter().any(|entry| entry.url == url) {
        return Err(invalid("relays-duplicate", url));
    }
    let tier = tier.unwrap_or_else(|| {
        view.entries
            .iter()
            .map(|entry| entry.tier)
            .max()
            .unwrap_or(0)
    });
    view.entries.push(RelaySettingEntry {
        url,
        token: token_value(token),
        quic_port: quic_port.or(fragment_port),
        tier,
        source: RelaySource::Settings,
        state: RelayState::Unknown,
        editable: true,
    });
    view.entries.sort_by_key(|entry| entry.tier);
    Ok(())
}

/// 三个可选字段的空值含义同协议里的 [`RelaySettingsAction::Update`]：令牌空着不动、给空串清掉；
/// 端口空着清掉（地址里的 `#端口` 在参数空着时生效）；档位空着不动。
fn update(
    view: &mut RelaySettingsView,
    url: String,
    token: Option<SecretString>,
    quic_port: Option<u16>,
    tier: Option<u8>,
) -> Result<(), CoreError> {
    editable(view)?;
    let (url, fragment_port) = validate_url(&url)?;
    let entry = entry_mut(view, &url)?;
    if !entry.editable {
        return Err(not_ready("relays-locked"));
    }
    if let Some(token) = token {
        entry.token = token_value(Some(token));
    }
    entry.quic_port = quic_port.or(fragment_port);
    if let Some(tier) = tier {
        entry.tier = tier;
    }
    view.entries.sort_by_key(|entry| entry.tier);
    Ok(())
}

fn remove(view: &mut RelaySettingsView, url: String) -> Result<(), CoreError> {
    editable(view)?;
    let (url, _) = validate_url(&url)?;
    let index = position(view, &url)?;
    if !view.entries[index].editable {
        return Err(not_ready("relays-locked"));
    }
    view.entries.remove(index);
    Ok(())
}

/// 挪到 `tier` 档、放在 `before` 前面；`before` 为空时放这一档的最后。
///
/// 表按档位排序，插进排完序的表里的最终位置，插完不再被排走：`before` 和自己同档时插在它前面；
/// `before` 就是自己时放这一档的最前；`before` 在别的档时按档位算，在更高档就放目标档的最后
/// （仍排在它前面），在更低档就放目标档的开头。
fn move_entry(
    view: &mut RelaySettingsView,
    url: String,
    tier: u8,
    before: Option<String>,
) -> Result<(), CoreError> {
    editable(view)?;
    let (url, _) = validate_url(&url)?;
    let index = position(view, &url)?;
    if !view.entries[index].editable {
        return Err(not_ready("relays-locked"));
    }
    let before = before
        .map(|before| validate_url(&before).map(|(url, _)| url))
        .transpose()?;
    if let Some(before) = &before
        && before != &url
        && position(view, before).is_err()
    {
        return Err(invalid("relays-before-missing", before.clone()));
    }
    let mut entry = view.entries.remove(index);
    entry.tier = tier;
    let at = match &before {
        None => tier_end(view, tier),
        Some(before) if before == &url => tier_start(view, tier),
        Some(before) => {
            let at = position(view, before).expect("checked above");
            match view.entries[at].tier.cmp(&tier) {
                Ordering::Equal => at,
                Ordering::Greater => tier_end(view, tier),
                Ordering::Less => tier_start(view, tier),
            }
        }
    };
    view.entries.insert(at, entry);
    debug_assert!(view.entries.is_sorted_by_key(|entry| entry.tier));
    Ok(())
}

/// 排好序的表里，第一项落在 `tier` 或更高档的位置。
fn tier_start(view: &RelaySettingsView, tier: u8) -> usize {
    view.entries.partition_point(|entry| entry.tier < tier)
}

/// 排好序的表里，`tier` 这一档的结束位置（第一项更高档的位置）。
fn tier_end(view: &RelaySettingsView, tier: u8) -> usize {
    view.entries.partition_point(|entry| entry.tier <= tier)
}

fn set_n0(view: &mut RelaySettingsView, on: bool) -> Result<(), CoreError> {
    editable(view)?;
    view.n0 = on;
    Ok(())
}

/// 整张表由环境变量固定时，哪一步都改不了。
fn editable(view: &RelaySettingsView) -> Result<(), CoreError> {
    if view.locked_by_env {
        Err(not_ready("relays-locked"))
    } else {
        Ok(())
    }
}

fn position(view: &RelaySettingsView, url: &str) -> Result<usize, CoreError> {
    view.entries
        .iter()
        .position(|entry| entry.url == url)
        .ok_or_else(|| invalid("relays-missing", url))
}

fn entry_mut<'a>(
    view: &'a mut RelaySettingsView,
    url: &str,
) -> Result<&'a mut RelaySettingEntry, CoreError> {
    view.entries
        .iter_mut()
        .find(|entry| entry.url == url)
        .ok_or_else(|| invalid("relays-missing", url))
}

/// 地址要做核心 `Relay::validate` 的同一套检查：https、不带 userinfo；地址里的空白先去掉，
/// DNS 根点（FQDN 尾点）也去掉，和核心的 `canonical_url` 写成同一种，同一个地址在两边才相等。
fn validate_url(text: &str) -> Result<(String, Option<u16>), CoreError> {
    let text = text.trim();
    let mut url = Url::parse(text).map_err(|_| invalid("relays-invalid-url", text.to_owned()))?;
    if url.scheme() != "https" {
        return Err(invalid("relays-needs-https", text.to_owned()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("relays-has-userinfo", text.to_owned()));
    }
    if let Some(domain) = url.domain() {
        let host = domain.trim_end_matches('.');
        if !host.is_empty() && url.host_str() != Some(host) {
            let host = host.to_owned();
            url.set_host(Some(&host))
                .expect("stripping the DNS root dot keeps a valid host");
        }
    }
    let relay =
        relay_from_item(url.as_str(), 0).map_err(|reason| invalid("relays-invalid-url", reason))?;
    Ok((
        wonton_tunnel::relays::relay_url_text(&relay.url),
        relay.quic_port,
    ))
}

fn token_value(token: Option<SecretString>) -> Option<SecretString> {
    token.and_then(|token| {
        let token = token.expose().trim();
        (!token.is_empty()).then(|| SecretString::new(token))
    })
}

#[cfg(test)]
#[path = "relays_tests.rs"]
mod relays_tests;
