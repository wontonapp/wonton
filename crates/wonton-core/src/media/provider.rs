//! 选媒体源提供方、按人签凭据（ADR-0113 第 2、6 节；规则见 `docs/adr/notes/0113-interfaces.md` 的
//! “选哪个提供方”“核心对提供方的凭据”）。
//!
//! 核心是 `media_source` 的使用方，凭据按人各一把：第一次替某人调某个提供方时现签，之后复用；人不在
//! 插件的可用范围里就不签，回 `permission_denied`。提供方是谁、在哪、怎么执行凭据命令由
//! [`ProviderDirectory`] 回答（生产里是插件管理），这里不认具体插件。

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wonton_protocol::media::provider::ProviderCapabilities;
use wonton_protocol::media::{MediaFailure, MediaFailureCode, MediaKind, MediaOperation};

use crate::http::HttpClient;
use crate::plugin::media_source::{MediaSourceClient, failure};
use crate::plugin::store::{create_private_dir, read_json, write_json};
use crate::plugin::upstreams::Endpoint;

/// 运行时自报的能力缓存多久。提供方升级换了能力，最多晚这么久生效。
const CAPABILITIES_TTL: Duration = Duration::from_secs(60);

/// 核心对一个人签的凭据名，同凭据命令里的 `{name}`。
pub(crate) fn credential_name(person: &str) -> String {
    format!("wonton-core.media.{person}")
}

/// 一个在跑的提供方，连同清单里自报的操作与种类（安装预检用的那份）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub id: String,
    pub operations: Vec<MediaOperation>,
    pub kinds: Vec<MediaKind>,
    /// 主人在能力设置里给 `media_source` 定的默认
    pub default: bool,
}

/// 目录里异步的一步。
pub(crate) type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T, MediaFailure>> + Send + 'a>>;

/// 提供方目录：哪些在跑、地址在哪、怎么签和收凭据。
pub(crate) trait ProviderDirectory: Send + Sync {
    /// 在跑的 `media_source` 提供方，标出主人定的默认。
    fn candidates(&self) -> Vec<Candidate>;
    fn endpoint<'a>(&'a self, provider: &'a str) -> Pending<'a, Endpoint>;
    /// 执行提供方的 `credentials.issue`，凭据值从标准输入给。
    fn issue<'a>(&'a self, provider: &'a str, name: &'a str, key: &'a str) -> Pending<'a, ()>;
    fn revoke<'a>(&'a self, provider: &'a str, name: &'a str) -> Pending<'a, ()>;
}

/// 选好的提供方和替这个人调它的客户端。
pub(crate) struct ProviderRoute {
    pub provider: String,
    pub person: String,
    pub(crate) client: MediaSourceClient,
}

impl std::fmt::Debug for ProviderRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRoute")
            .field("provider", &self.provider)
            .field("person", &self.person)
            .finish_non_exhaustive()
    }
}

pub(crate) struct MediaProviders {
    directory: Arc<dyn ProviderDirectory>,
    http: HttpClient,
    /// 每个提供方一个文件：人 → 凭据。目录只有核心读得到。
    keys_dir: PathBuf,
    issuing: tokio::sync::Mutex<()>,
    capabilities: Mutex<HashMap<String, (Instant, ProviderCapabilities)>>,
}

impl MediaProviders {
    pub(crate) fn new(
        directory: Arc<dyn ProviderDirectory>,
        http: HttpClient,
        keys_dir: PathBuf,
    ) -> Self {
        Self {
            directory,
            http,
            keys_dir,
            issuing: tokio::sync::Mutex::new(()),
            capabilities: Mutex::new(HashMap::new()),
        }
    }

    /// 替 `person` 选一个能做 `operation` 的提供方；`kind` 为空时不看种类（上传）。`source` 是请求带的来源
    /// （条目的 `source`、上传的 `TaskOptions.source`、`?source=`）。`allowed` 判这个人能不能用某个插件。
    ///
    /// 规则见 `notes/0113-interfaces.md`“选哪个提供方”：带来源用它；没带用主人定的默认；只有一家用它；
    /// 几家又没定默认回 `source_required`。选中的那家再按运行时自报核对，对不上不换别家。
    pub(crate) async fn route(
        &self,
        person: &str,
        allowed: impl Fn(&str) -> bool,
        operation: &MediaOperation,
        kind: Option<&MediaKind>,
        source: Option<&str>,
    ) -> Result<ProviderRoute, MediaFailure> {
        let chosen = self.choose(person, &allowed, operation, kind, source)?;
        let route = self.client(person, &chosen).await?;
        let caps = self.capabilities_of(&route).await?;
        if caps.operations.contains(operation) && kind.is_none_or(|k| caps.kinds.contains(k)) {
            return Ok(route);
        }
        Err(failure(
            MediaFailureCode::PluginUnavailable,
            format!(
                "媒体源插件 {chosen} 运行时没说能{}",
                wanted(operation, kind)
            ),
        ))
    }

    fn choose(
        &self,
        person: &str,
        allowed: &impl Fn(&str) -> bool,
        operation: &MediaOperation,
        kind: Option<&MediaKind>,
        source: Option<&str>,
    ) -> Result<String, MediaFailure> {
        let fits = |c: &Candidate| {
            c.operations.contains(operation) && kind.is_none_or(|k| c.kinds.contains(k))
        };
        let candidates = self.directory.candidates();
        if let Some(source) = source {
            let named = candidates.iter().find(|c| c.id == source);
            // 先看授权：用不了这家的人不该从回答里知道它能做什么。
            return match named {
                Some(_) if !allowed(source) => Err(denied(person, source)),
                Some(c) if !fits(c) => Err(failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源插件 {source} 不能{}", wanted(operation, kind)),
                )),
                Some(_) => Ok(source.to_owned()),
                None => Err(failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源插件 {source} 没在跑"),
                )),
            };
        }
        let fitting: Vec<&Candidate> = candidates.iter().filter(|c| fits(c)).collect();
        let usable: Vec<&Candidate> = fitting.iter().copied().filter(|c| allowed(&c.id)).collect();
        if let Some(default) = usable.iter().find(|c| c.default) {
            return Ok(default.id.clone());
        }
        match (usable.as_slice(), fitting.first()) {
            ([only], _) => Ok(only.id.clone()),
            ([], Some(first)) => Err(denied(person, &first.id)),
            ([], None) => Err(failure(
                MediaFailureCode::PluginUnavailable,
                format!("没有在跑的媒体源插件能{}", wanted(operation, kind)),
            )),
            (several, _) => Err(failure(
                MediaFailureCode::SourceRequired,
                format!(
                    "能{}的媒体源有 {} 家，主人没定默认，请选一家",
                    wanted(operation, kind),
                    several.len()
                ),
            )),
        }
    }

    /// 替 `person` 调已经记下的那个提供方（任务、会话建好时记的）。和点名来源一样核对清单和运行时能力：
    /// 记下之后插件升级去掉了这项能力，回 `plugin_unavailable`，不等提供方回 404 被当成条目没了，也不换别家。
    pub(crate) async fn route_to(
        &self,
        person: &str,
        allowed: impl Fn(&str) -> bool,
        provider: &str,
        operation: &MediaOperation,
        kind: Option<&MediaKind>,
    ) -> Result<ProviderRoute, MediaFailure> {
        self.route(person, allowed, operation, kind, Some(provider))
            .await
    }

    /// 收回这个人在所有提供方上的凭据：先让提供方注销上游凭据，再执行它的 `credentials.revoke`，最后删掉
    /// 核心存的这把。哪一家失败就停在哪一家、保留它的凭据，返回失败；已收回的不再出现。
    pub(crate) async fn revoke_person(&self, person: &str) -> Result<(), MediaFailure> {
        let name = credential_name(person);
        let _issuing = self.issuing.lock().await;
        for provider in self.providers_with_key(person)? {
            let route = self.client_locked(person, &provider).await?;
            route.client.revoke(person, &name).await?;
            self.directory.revoke(&provider, &name).await?;
            self.forget_key(&provider, person)?;
        }
        Ok(())
    }

    /// 已签过就直接用；没签过才排队签，两个请求同时替同一个人第一次调同一家只签一次。
    async fn client(&self, person: &str, provider: &str) -> Result<ProviderRoute, MediaFailure> {
        if let Some(key) = self.keys(provider)?.get(person) {
            return self.build(person, provider, key.clone()).await;
        }
        let _issuing = self.issuing.lock().await;
        self.client_locked(person, provider).await
    }

    /// 调用方要持着 `issuing`。
    async fn client_locked(
        &self,
        person: &str,
        provider: &str,
    ) -> Result<ProviderRoute, MediaFailure> {
        let mut keys = self.keys(provider)?;
        let key = match keys.get(person) {
            Some(key) => key.clone(),
            None => {
                let key = crate::secret::random_token();
                self.directory
                    .issue(provider, &credential_name(person), &key)
                    .await?;
                keys.insert(person.to_owned(), key.clone());
                self.save_keys(provider, &keys)?;
                key
            }
        };
        self.build(person, provider, key).await
    }

    async fn build(
        &self,
        person: &str,
        provider: &str,
        key: String,
    ) -> Result<ProviderRoute, MediaFailure> {
        let endpoint = self.directory.endpoint(provider).await?;
        Ok(ProviderRoute {
            provider: provider.to_owned(),
            person: person.to_owned(),
            client: MediaSourceClient::new(endpoint, key, self.http.clone())?,
        })
    }

    async fn capabilities_of(
        &self,
        route: &ProviderRoute,
    ) -> Result<ProviderCapabilities, MediaFailure> {
        if let Some((at, caps)) = self
            .capabilities
            .lock()
            .expect("能力缓存锁中毒")
            .get(&route.provider)
            && at.elapsed() < CAPABILITIES_TTL
        {
            return Ok(caps.clone());
        }
        let caps = route.client.capabilities(&route.person).await?;
        self.capabilities
            .lock()
            .expect("能力缓存锁中毒")
            .insert(route.provider.clone(), (Instant::now(), caps.clone()));
        Ok(caps)
    }

    fn key_file(&self, provider: &str) -> PathBuf {
        self.keys_dir.join(format!("{provider}.json"))
    }

    fn keys(&self, provider: &str) -> Result<BTreeMap<String, String>, MediaFailure> {
        read_json(&self.key_file(provider))
            .map(Option::unwrap_or_default)
            .map_err(|e| key_store_failure(&e))
    }

    fn save_keys(
        &self,
        provider: &str,
        keys: &BTreeMap<String, String>,
    ) -> Result<(), MediaFailure> {
        create_private_dir(&self.keys_dir).map_err(|e| key_store_failure(&e))?;
        write_json(&self.key_file(provider), keys).map_err(|e| key_store_failure(&e))
    }

    fn forget_key(&self, provider: &str, person: &str) -> Result<(), MediaFailure> {
        let mut keys = self.keys(provider)?;
        keys.remove(person);
        self.save_keys(provider, &keys)
    }

    fn providers_with_key(&self, person: &str) -> Result<Vec<String>, MediaFailure> {
        let entries = match std::fs::read_dir(&self.keys_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(key_store_failure(&e)),
        };
        let mut providers = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| key_store_failure(&e))?;
            let name = entry.file_name();
            let Some(provider) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            if self.keys(provider)?.contains_key(person) {
                providers.push(provider.to_owned());
            }
        }
        providers.sort();
        Ok(providers)
    }
}

fn denied(person: &str, provider: &str) -> MediaFailure {
    failure(
        MediaFailureCode::PermissionDenied,
        format!("{person} 不能用媒体源插件 {provider}"),
    )
}

fn key_store_failure(error: &dyn std::fmt::Display) -> MediaFailure {
    failure(
        MediaFailureCode::UpstreamError,
        format!("核心读写媒体源凭据失败：{error}"),
    )
}

fn operation_label(operation: &MediaOperation) -> &str {
    match operation {
        MediaOperation::Upload => "上传",
        MediaOperation::Download => "下载",
        MediaOperation::Playback => "播放",
        MediaOperation::List => "列条目",
        MediaOperation::Library => "浏览相册",
        MediaOperation::Other(raw) => raw,
    }
}

/// “上传”“播放视频”这样的一句，拼进失败原文。
fn wanted(operation: &MediaOperation, kind: Option<&MediaKind>) -> String {
    format!(
        "{}{}",
        operation_label(operation),
        kind.map(kind_label).unwrap_or_default()
    )
}

fn kind_label(kind: &MediaKind) -> String {
    match kind {
        MediaKind::Photo => "照片".into(),
        MediaKind::Video => "视频".into(),
        MediaKind::Audio => "音乐".into(),
        MediaKind::Other(raw) => format!("（{raw}）"),
    }
}
