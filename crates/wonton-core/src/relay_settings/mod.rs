//! 中继表设置（ADR-0048）：内置默认、签名的列表、主人配置三层合并，主人能在 App 里改，改完当场换掉
//! endpoint 的中继。
//!
//! 优先级从低到高：编译进核心的内置默认（`WONTON_BUILTIN_RELAYS`，部署时经构建参数给）→ 签名的
//! 列表（`WONTON_RELAY_LIST_URL`，验签不过就不用，只留一条提示）→ 主人配置。同一地址后面的层盖过
//! 前面的层；`!URL` 是删除标记。
//!
//! 主人配置两个来源：`WONTON_RELAYS` 环境变量设了就只认它、设置页只读（部署方固定）；没设就读数据
//! 目录里的 `relays.json`（`WONTON_RELAYS_FILE` 指定，`none` 关掉），设置页写它。
//!
//! 内置的 n0 公共中继默认总在最后一档兜底（ADR-0048）；主人配置写 `no-n0`（环境变量）或
//! `"n0": false`（文件）就去掉。
//!
//! 换表当场生效：旧表和新表出现过的地址全部撤出 map，只放新表最优先一档，回退任务换表从头算。
//! 地址或令牌变了靠先删再插让 iroh 重新握手（iroh#4642：换掉已有的项不会重连）。

mod file;
mod signed;
mod view;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use iroh::{Endpoint, RelayUrl};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use wonton_protocol::{
    RelaySettingEntry, RelaySettingUpdate, RelaySettingsSnapshot, RelaySettingsUpdate, RelaySource,
    SecretString,
};
use wonton_tunnel::relays::{
    Relay, RelayTable, RelayToken, TierFallbackHandle, canonical_url, is_public_relay, n0_relays,
    relay_from_item, relay_url_text,
};

use crate::config::Config;

/// 一张表最多这么多项。够用（n0 一组三台、我们自己的几台、主人自己的几台），也挡住粘贴进来的
/// 巨大列表每拍都扫一遍。
const MAX_RELAYS: usize = 64;

/// 打开设置时的输入：编译进来的默认表、签名列表地址和公钥、环境变量和出网代理。
pub struct Startup {
    pub(crate) builtin: RelayTable,
    pub(crate) signed_url: Option<String>,
    pub(crate) signed_pubkey: Option<String>,
    pub(crate) env: Option<OwnerLayer>,
    pub(crate) file: Option<PathBuf>,
    /// 读签名列表时的出网代理；`WONTON_EGRESS_PROXY` 或安装时检测到的那个。
    pub(crate) proxy: Option<String>,
}

impl Startup {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            builtin: cfg.builtin_relays.clone(),
            signed_url: cfg.relay_list_url.clone(),
            signed_pubkey: cfg.relay_list_pubkey.clone(),
            env: cfg.relays.clone(),
            file: cfg.relay_settings_file.clone(),
            proxy: cfg
                .egress_proxy
                .as_ref()
                .or(cfg.egress_proxy_detected.as_ref())
                .map(|proxy| proxy.text().to_owned()),
        }
    }
}

/// 主人配置的一层：表本身，以及要不要 n0 兜底。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerLayer {
    relays: RelayTable,
    n0: bool,
}

impl OwnerLayer {
    /// 解析 `WONTON_RELAYS`：和 [`RelayTable::parse`] 同一种写法，多一个 `no-n0` 关键字（放在
    /// 哪一档都行，去掉内置的 n0 兜底）。
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Err("WONTON_RELAYS 是空的".to_owned());
        }
        let mut n0 = true;
        let mut relays: Vec<Relay> = Vec::new();
        let mut remove: Vec<RelayUrl> = Vec::new();
        for (index, tier_text) in text.split(';').enumerate() {
            let tier = u8::try_from(index).map_err(|_| "档位太多".to_owned())?;
            for item in tier_text.split(',') {
                let item = item.trim();
                match item {
                    "" => return Err(format!("第 {} 档里有空的中继项", index + 1)),
                    "no-n0" => {
                        n0 = false;
                        continue;
                    }
                    "n0" => relays.extend(n0_relays(tier)),
                    _ => {
                        if let Some(url) = item.strip_prefix('!') {
                            if url == "n0" {
                                remove.extend(n0_relays(0).map(|relay| relay.url));
                            } else {
                                remove.push(relay_from_item(url, tier)?.url);
                            }
                        } else {
                            relays.push(relay_from_item(item, tier)?);
                        }
                    }
                }
            }
        }
        Ok(Self {
            relays: RelayTable::from_relays(relays).with_remove(remove),
            n0,
        })
    }
}

/// `relays.json` 的内容。`version` 留给以后改格式，现在只认 1。
#[derive(Debug, Default, Serialize, Deserialize)]
struct OwnerFile {
    #[serde(default)]
    version: u32,
    /// 默认 true；文件里没写时保留 n0 兜底。
    #[serde(default = "yes")]
    n0: bool,
    #[serde(default)]
    relays: Vec<OwnerEntry>,
    #[serde(default)]
    remove: Vec<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
struct OwnerEntry {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quic_port: Option<u16>,
    #[serde(default)]
    tier: u8,
}

impl OwnerFile {
    fn to_layer(&self) -> Result<OwnerLayer, String> {
        if self.version > 1 {
            return Err(format!("版本 {} 比这个核心新", self.version));
        }
        if self.relays.len() > MAX_RELAYS {
            return Err(format!("中继超过 {MAX_RELAYS} 条"));
        }
        let mut relays = Vec::with_capacity(self.relays.len());
        for entry in &self.relays {
            relays.push(parse_entry(
                &entry.url,
                entry.token.as_deref(),
                entry.quic_port,
                entry.tier,
            )?);
        }
        let remove = self
            .remove
            .iter()
            .map(|url| relay_from_item(url, 0).map(|relay| relay.url))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OwnerLayer {
            relays: RelayTable::from_relays(relays).with_remove(remove),
            n0: self.n0,
        })
    }

    fn from_layer(layer: &OwnerLayer, remove: Vec<RelayUrl>) -> Self {
        Self {
            version: 1,
            n0: layer.n0,
            relays: layer
                .relays
                .relays()
                .iter()
                .map(|relay| OwnerEntry {
                    url: relay_url_text(&relay.url),
                    token: relay.token.as_ref().map(|token| token.expose().to_owned()),
                    quic_port: relay.quic_port,
                    tier: relay.tier,
                })
                .collect(),
            remove: remove.into_iter().map(|url| relay_url_text(&url)).collect(),
        }
    }
}

fn parse_entry(
    url: &str,
    token: Option<&str>,
    quic_port: Option<u16>,
    tier: u8,
) -> Result<Relay, String> {
    let mut relay = relay_from_item(url, tier)?;
    if let Some(token) = token {
        relay.token = Some(RelayToken::new(token));
    }
    relay.quic_port = quic_port.or(relay.quic_port);
    relay.validate()?;
    Ok(relay)
}

/// 合并出来的表和每个地址的来源，渲染设置页时用。
#[derive(Debug)]
struct State {
    /// 内置和签名两层。改表时把主人配置重新盖在它们上面。
    base: RelayTable,
    /// 签名列表里出现的地址。
    signed: RelayTable,
    /// 正在起作用的主人配置。
    owner: OwnerLayer,
    /// 合并后的表。
    resolved: RelayTable,
    /// 每个地址是谁给的。
    sources: BTreeMap<RelayUrl, RelaySource>,
    locked_by_env: bool,
    warnings: Vec<String>,
    file: Option<PathBuf>,
    endpoint: Option<Endpoint>,
    fallback: Option<TierFallbackHandle>,
    /// 我们要求 endpoint 持有的中继地址，attach 时记第一档、换表时按撤/插更新。
    /// 回退任务自己插进来的高档不在这里（它在同一张表上操作），换表时按 `old ∪ new` 清。
    mapped: std::collections::BTreeSet<RelayUrl>,
}

/// 一台服务器现在的中继表。
#[derive(Debug)]
pub struct RelaySettings {
    state: RwLock<State>,
    /// 改表一次一个：写文件和换 endpoint 的中继之间不能插进第二次修改。
    apply: Mutex<()>,
}

/// 打开设置失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("读 {path}：{source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} 不合法：{reason}")]
    Parse { path: String, reason: String },
}

/// 改表失败的原因。`Locked` 和 `Invalid` 是主人能改的，其余是这次的运行环境问题。
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("中继表由环境变量固定，改不了")]
    Locked,
    #[error("{0}")]
    Invalid(String),
    #[error("写中继表：{0}")]
    Store(String),
    #[error("换中继：{0}")]
    Apply(String),
}

impl RelaySettings {
    /// 读主人的配置文件、取签名列表，合成当前的表。签名列表取不到、验不过只留提示，不挡启动。
    pub async fn open(startup: Startup) -> Result<Self, LoadError> {
        let mut warnings = Vec::new();
        let signed = match (&startup.signed_url, &startup.signed_pubkey) {
            (Some(url), Some(pubkey)) => {
                match signed::fetch(url, pubkey, startup.proxy.as_deref()).await {
                    Ok(table) => table,
                    Err(reason) => {
                        warnings.push(format!("签名的中继列表没用上：{reason}"));
                        RelayTable::default()
                    }
                }
            }
            (Some(_), None) => {
                warnings.push(
                    "设了 WONTON_RELAY_LIST_URL 但没有 WONTON_RELAY_LIST_PUBKEY，签名的中继列表没用上"
                        .to_owned(),
                );
                RelayTable::default()
            }
            _ => RelayTable::default(),
        };
        let base = signed.overlay(&startup.builtin);
        let owner = match &startup.env {
            Some(env) => env.clone(),
            None => match &startup.file {
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(text) => {
                        if let Some(warning) = file::permission_warning(path) {
                            warnings.push(warning);
                        }
                        file::parse(path, &text)?
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => OwnerLayer {
                        relays: RelayTable::default(),
                        n0: true,
                    },
                    Err(source) => {
                        return Err(LoadError::Read {
                            path: path.display().to_string(),
                            source,
                        });
                    }
                },
                None => OwnerLayer {
                    relays: RelayTable::default(),
                    n0: true,
                },
            },
        };
        let resolved = resolve(&base, &owner);
        let sources = sources(&startup.builtin, &signed, &owner, startup.env.is_some());
        Ok(Self {
            state: RwLock::new(State {
                base,
                signed,
                owner,
                resolved,
                sources,
                locked_by_env: startup.env.is_some(),
                warnings,
                file: startup.file,
                endpoint: None,
                fallback: None,
                mapped: std::collections::BTreeSet::new(),
            }),
            apply: Mutex::new(()),
        })
    }

    /// 只拿一张表建出来，给测试和不读配置的调用方；没有 endpoint、不写文件。
    #[cfg(test)]
    pub(crate) fn for_table(table: RelayTable) -> Self {
        Self {
            state: RwLock::new(State {
                base: table.clone(),
                signed: RelayTable::default(),
                owner: OwnerLayer {
                    relays: RelayTable::default(),
                    n0: true,
                },
                sources: table
                    .relays()
                    .iter()
                    .map(|relay| (relay.url.clone(), RelaySource::Builtin))
                    .collect(),
                resolved: table,
                locked_by_env: false,
                warnings: Vec::new(),
                file: None,
                endpoint: None,
                fallback: None,
                mapped: std::collections::BTreeSet::new(),
            }),
            apply: Mutex::new(()),
        }
    }

    /// endpoint 起来了以后交进来，之后改表才换得动中继。
    pub fn attach(&self, endpoint: Endpoint, fallback: TierFallbackHandle) {
        let mut state = self.state.write().expect("中继表锁中毒");
        state.endpoint = Some(endpoint);
        state.fallback = Some(fallback);
        // bind 只放了最优先一档；记录它是为了换表时对得上，高档由回退任务自己插。
        state.mapped = state
            .resolved
            .tier(0)
            .iter()
            .map(|relay| relay.url.clone())
            .collect();
    }

    /// 只给测试：把这张表标成由环境变量固定，`save` 一律回 [`SaveError::Locked`]。
    #[cfg(test)]
    pub(crate) fn lock_for_test(&self) {
        self.state.write().expect("中继表锁中毒").locked_by_env = true;
    }

    /// 当前表。起 endpoint 时用它建最优先一档。
    pub fn table(&self) -> RelayTable {
        self.state.read().expect("中继表锁中毒").resolved.clone()
    }

    /// 设置页要的一整份表，带上每台中继现在的连接状态。
    pub fn snapshot(&self) -> RelaySettingsSnapshot {
        let state = self.state.read().expect("中继表锁中毒");
        let statuses = state
            .endpoint
            .as_ref()
            .map(view::statuses)
            .unwrap_or_default();
        let entries = state
            .resolved
            .relays()
            .iter()
            .map(|relay| RelaySettingEntry {
                url: relay_url_text(&relay.url),
                token: relay
                    .token
                    .as_ref()
                    .map(|token| SecretString::new(token.expose())),
                quic_port: relay.quic_port,
                tier: relay.tier,
                source: state
                    .sources
                    .get(&relay.url)
                    .copied()
                    .unwrap_or(RelaySource::Settings),
                state: view::state_of(&statuses, &relay.url),
                editable: !state.locked_by_env,
            })
            .collect();
        RelaySettingsSnapshot {
            entries,
            n0: state.owner.n0,
            locked_by_env: state.locked_by_env,
            warnings: state.warnings.clone(),
        }
    }

    /// 存下整张表：写配置文件、换 endpoint 的中继、让回退任务从头算。返回新的设置页。
    pub async fn save(
        &self,
        update: RelaySettingsUpdate,
    ) -> Result<RelaySettingsSnapshot, SaveError> {
        let _applying = self.apply.lock().await;
        let (locked, base, file) = {
            let state = self.state.read().expect("中继表锁中毒");
            (state.locked_by_env, state.base.clone(), state.file.clone())
        };
        if locked {
            return Err(SaveError::Locked);
        }
        if update.entries.len() > MAX_RELAYS {
            return Err(SaveError::Invalid(format!(
                "中继最多 {MAX_RELAYS} 条，现在是 {}",
                update.entries.len()
            )));
        }
        let mut relays = Vec::with_capacity(update.entries.len());
        let mut seen = BTreeMap::new();
        for entry in &update.entries {
            let relay = owner_entry(entry)?;
            if let Some(previous) = seen.insert(relay.url.clone(), relay.tier) {
                return Err(SaveError::Invalid(format!(
                    "{} 出现了两次（档位 {previous} 和 {}）",
                    relay_url_text(&relay.url),
                    relay.tier
                )));
            }
            relays.push(relay);
        }
        let owner = OwnerLayer {
            relays: RelayTable::from_relays(relays),
            n0: update.n0,
        };
        let (old, file_to_write) = {
            let state = self.state.read().expect("中继表锁中毒");
            let old = state.resolved.clone();
            let file_to_write = file.as_ref().map(|path| {
                let keep: std::collections::BTreeSet<_> = owner
                    .relays
                    .relays()
                    .iter()
                    .map(|relay| relay.url.clone())
                    .collect();
                // n0 兜底那一组可能不在 base 里（base 只有内置和签名），不把它们算进 lower
                // 的话，删掉的 n0 地址没记进 remove，下次启动会被自动加回来。表里一项 n0 都
                // 没有时当作「就要 n0 兜底」，把 n0 从 lower 里去掉，这样把 n0 关掉再打开能
                // 整组补回来。
                let mut lower = lower_urls(&state.base);
                let draft_has_n0 = owner
                    .relays
                    .relays()
                    .iter()
                    .any(|relay| is_public_relay(&relay.url));
                if update.n0 {
                    if draft_has_n0 {
                        lower.extend(n0_relays(0).map(|relay| relay.url));
                    } else {
                        lower.retain(|url| !is_public_relay(url));
                    }
                }
                let remove = lower
                    .into_iter()
                    .filter(|url| !keep.contains(url))
                    .collect();
                (path.clone(), OwnerFile::from_layer(&owner, remove))
            });
            (old, file_to_write)
        };
        let resolved = resolve(&base, &owner);
        // 先把 endpoint 换到新表；失败就换回旧表，状态和文件都还没动。
        if let Err(error) = self.sync_endpoint(&old, &resolved).await {
            self.rollback_endpoint(&resolved, &old).await;
            return Err(SaveError::Apply(error.to_string()));
        }
        if let Some((path, file)) = file_to_write
            && let Err(error) = file::write(&path, &file)
        {
            self.rollback_endpoint(&resolved, &old).await;
            return Err(SaveError::Store(error.to_string()));
        }
        // endpoint 已经换好了才提交状态：中途失败时内存里仍是旧表，重启再按文件来。
        {
            let mut state = self.state.write().expect("中继表锁中毒");
            state.owner = owner;
            state.resolved = resolved.clone();
            state.sources = sources_of(&state.base, &state.signed, &state.owner);
            // 文件刚被写成 0600，之前那条 0644 的权限警告要重新算，不能留在设置页上。
            if let Some(path) = &file {
                let prefix = format!("{} 的权限", path.display());
                state
                    .warnings
                    .retain(|warning| !warning.starts_with(&prefix));
                if let Some(warning) = file::permission_warning(path) {
                    state.warnings.push(warning);
                }
            }
        }
        Ok(self.snapshot())
    }

    /// 把 endpoint 的中继换成 `new`：先把两张表里出现过的地址全部撤出 map（回退任务插进来的高档
    /// 也在 `old` 里），只把新表最优先一档放回去，再让回退任务按新表从头算。
    ///
    /// 只同步第一档会留下旧高档：删掉/换了令牌的档 1、2 一直留在 map 里继续被拨。
    async fn sync_endpoint(&self, old: &RelayTable, new: &RelayTable) -> anyhow::Result<()> {
        let (endpoint, fallback_attached) = {
            let state = self.state.read().expect("中继表锁中毒");
            (state.endpoint.clone(), state.fallback.as_ref().map(|_| ()))
        };
        let Some(endpoint) = endpoint else {
            return Ok(());
        };
        if endpoint.is_closed() {
            anyhow::bail!("隧道 endpoint 已关闭，中继表换不了");
        }
        for relay in old.relays().iter().chain(new.relays()) {
            endpoint.remove_relay(&relay.url).await;
            let mut state = self.state.write().expect("中继表锁中毒");
            state.mapped.remove(&relay.url);
        }
        if new.tier_count() > 0 {
            for relay in new.tier(0) {
                endpoint
                    .insert_relay(relay.url.clone(), Arc::new(relay.relay_config()))
                    .await;
                let mut state = self.state.write().expect("中继表锁中毒");
                state.mapped.insert(relay.url.clone());
            }
        }
        if fallback_attached.is_some() {
            let state = self.state.read().expect("中继表锁中毒");
            if let Some(fallback) = state.fallback.as_ref() {
                fallback.set_table(new.clone());
            }
        }
        Ok(())
    }

    /// 换表失败时把 endpoint 换回旧表。换不回来只记日志：内存里仍是旧表，重启会按文件恢复。
    async fn rollback_endpoint(&self, attempted: &RelayTable, old: &RelayTable) {
        if let Err(error) = self.sync_endpoint(attempted, old).await {
            tracing::warn!(%error, "中继表：换表失败后回滚 endpoint 也失败");
        }
    }
}

fn owner_entry(entry: &RelaySettingUpdate) -> Result<Relay, SaveError> {
    let relay = parse_entry(
        &entry.url,
        entry.token.as_ref().map(SecretString::expose),
        entry.quic_port,
        entry.tier,
    )
    .map_err(SaveError::Invalid)?;
    if relay.quic_port == Some(0) {
        return Err(SaveError::Invalid(format!(
            "{}：QUIC 端口不能是 0",
            relay_url_text(&relay.url)
        )));
    }
    relay.validate().map_err(SaveError::Invalid)?;
    Ok(relay)
}

fn sources(
    builtin: &RelayTable,
    signed: &RelayTable,
    owner: &OwnerLayer,
    from_env: bool,
) -> BTreeMap<RelayUrl, RelaySource> {
    let mut sources = sources_of(builtin, signed, owner);
    if from_env {
        for relay in owner.relays.relays() {
            sources.insert(canonical_url(relay.url.clone()), RelaySource::Env);
        }
    }
    sources
}

fn sources_of(
    builtin: &RelayTable,
    signed: &RelayTable,
    owner: &OwnerLayer,
) -> BTreeMap<RelayUrl, RelaySource> {
    let mut sources = BTreeMap::new();
    // n0 兜底是内置来源，不管它从哪一层被加回来。
    for relay in n0_relays(0) {
        sources.insert(relay.url, RelaySource::Builtin);
    }
    for relay in builtin.relays() {
        sources.insert(canonical_url(relay.url.clone()), RelaySource::Builtin);
    }
    for relay in signed.relays() {
        sources.insert(canonical_url(relay.url.clone()), RelaySource::Signed);
    }
    for relay in owner.relays.relays() {
        sources.insert(canonical_url(relay.url.clone()), RelaySource::Settings);
    }
    sources
}

/// 底层来源里出现过的地址。存主人的配置文件时，这些地址里没被主人保留的记为删除。
fn lower_urls(base: &RelayTable) -> Vec<RelayUrl> {
    let mut urls: Vec<RelayUrl> = base
        .relays()
        .iter()
        .map(|relay| relay.url.clone())
        .collect();
    urls.extend(base.remove().iter().cloned());
    urls.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    urls.dedup();
    urls
}

/// 内置、签名两层当底，主人配置盖上去；n0 关掉时把它从结果里删掉，除非主人自己显式列了。
fn resolve(base: &RelayTable, owner: &OwnerLayer) -> RelayTable {
    let table = owner.relays.overlay(base);
    if owner.n0 {
        return table.with_n0_last(true);
    }
    let explicit: std::collections::BTreeSet<RelayUrl> = owner
        .relays
        .relays()
        .iter()
        .map(|relay| relay.url.clone())
        .collect();
    let n0: Vec<RelayUrl> = n0_relays(0)
        .map(|relay| relay.url)
        .filter(|url| !explicit.contains(url))
        .collect();
    table.without_urls(&n0)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
