//! 已安装插件的记录：每个插件一个 JSON 文件，核心重启后据此恢复路由和 OIDC 客户端。
//!
//! 记录里有整个插件包（compose 原文和它引用的文件）和 OIDC 客户端密钥，文件权限设为 0600。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rustix::io::Errno;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use wonton_protocol::{LanChoice, PluginLimits, PluginWriter};

use super::bots::BotStore;
use super::egress::settings::StoredEgress;
use super::github_source::Origin;
use super::languages::Languages;
use super::manifest::{Manifest, Permission, PluginId};
use super::models::ModelKeyStore;
use super::package_files::PackageFiles;
use super::preinstall::PreinstallMarks;
use super::save_grants::SaveGrantStore;
use super::secrets::SecretStore;
use super::tool_policy::ToolPolicy;

/// 一项可选权限现在开着，以及是谁、什么时候打开的（ADR-0057）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnabledPermission {
    pub permission: Permission,
    /// Unix 秒。
    pub since: u64,
    pub opened_by: OpenedBy,
}

/// 打开一项可选权限的人。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum OpenedBy {
    Owner,
    Bot { plugin_id: String, title: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub id: PluginId,
    pub version: String,
    /// 原始 compose 文本。
    pub source: String,
    /// 插件包里被引用的文件，供审计、恢复和升级比对。JSON 里是 base64。
    #[serde(
        default,
        skip_serializing_if = "PackageFiles::is_empty",
        with = "base64_map"
    )]
    pub files: PackageFiles,
    pub granted_permissions: Vec<Permission>,
    /// 主人或 Bot 打开了的可选项（ADR-0057）；关着的可选项不在这里。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_permissions: Vec<EnabledPermission>,
    #[serde(default, skip_serializing_if = "limits_empty")]
    pub limits: PluginLimits,
    /// 开关 `lan_webui`（见 [`super::lan::webui`]），默认关。
    #[serde(default)]
    pub lan_webui: bool,
    /// 宿主网络网页和 MCP 服务的中继（见 [`super::host_relay`]）。
    #[serde(default)]
    pub relays: Vec<super::host_relay::HostRelay>,
    /// 装好后直连宿主网络的网页连不上：本机防火墙可能挡住了局域网设备。
    #[serde(default)]
    pub firewalled: bool,
    /// 容器的环境变量里有代理变量（ADR-0023）：和现在的设置对不上时要重建。
    #[serde(default)]
    pub proxied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_secret: Option<String>,
    pub installed_at: u64,
    /// 按来源装的插件从哪里来（来源、tag、提交）；上传的和预装的为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// 写这个插件的 Agent（ADR-0033）；只有 Agent 经 outbox 装的插件才有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_by: Option<PluginWriter>,
    /// 主人打开的“每次都问”：这个 Agent 每次更新都发卡片。只在 written_by 有值时有意义。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ask_every_update: bool,
    /// 主人开放给家人了（ADR-0045）：家人的设备能打开它的网页，家人的插件列表里有它。默认关。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub family: bool,
}

impl InstalledRecord {
    /// 记录里开着的可选权限。
    pub fn enabled(&self) -> Vec<Permission> {
        self.enabled_permissions
            .iter()
            .map(|item| item.permission.clone())
            .collect()
    }

    /// 这份记录对 `manifest` 的生效集：必需权限加上开着的可选项。
    pub fn effective_permissions(&self, manifest: &Manifest) -> Vec<Permission> {
        manifest.effective_permissions(&self.enabled())
    }
}

fn limits_empty(limits: &PluginLimits) -> bool {
    *limits == PluginLimits::default()
}

/// 解析请求里的权限写法。
pub fn parse_permissions(
    values: &[String],
) -> Result<Vec<Permission>, super::manifest::ManifestError> {
    values
        .iter()
        .map(|value| value.parse::<Permission>())
        .collect()
}

/// 打开的可选项里的权限名。
pub fn permission_set(items: &[EnabledPermission]) -> Vec<Permission> {
    items.iter().map(|item| item.permission.clone()).collect()
}

/// 解析请求里打开了的可选项，带上打开的时间和人。
pub fn parse_enabled(
    values: &[String],
    opened_by: OpenedBy,
    since: u64,
) -> Result<Vec<EnabledPermission>, super::manifest::ManifestError> {
    values
        .iter()
        .map(|value| {
            Ok(EnabledPermission {
                permission: value.parse::<Permission>()?,
                since,
                opened_by: opened_by.clone(),
            })
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("读写插件记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("插件记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
}

/// 一个记录文件。ID 取自文件名，所以记录读不出或已损坏时也知道是哪个插件。
#[derive(Debug)]
pub struct RecordFile {
    pub id: PluginId,
    pub record: Result<InstalledRecord, StoreError>,
}

pub struct Store {
    dir: PathBuf,
    secrets: SecretStore,
    bots: BotStore,
    model_keys: ModelKeyStore,
    preinstalled: PreinstallMarks,
    save_grants: SaveGrantStore,
    tool_policy: ToolPolicy,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        create_private_dir(&dir)?;
        // 记录目录顶层的 `*.json` 都当插件记录，这个文件要放进子目录。
        let tool_policy = dir.join("tool-policy");
        create_private_dir(&tool_policy)?;
        Ok(Self {
            secrets: SecretStore::new(dir.join("secrets")),
            bots: BotStore::new(dir.join("bots")),
            model_keys: ModelKeyStore::new(dir.join("model-keys")),
            preinstalled: PreinstallMarks::new(dir.join("preinstalled")),
            save_grants: SaveGrantStore::new(dir.join("save-grants")),
            tool_policy: ToolPolicy::load(tool_policy.join("policy.json")),
            dir,
        })
    }

    /// 主人对确认的放宽，在记录目录下的 `tool-policy/` 里。
    pub fn tool_policy(&self) -> &ToolPolicy {
        &self.tool_policy
    }

    /// 插件密钥库，在记录目录下的 `secrets/` 里。和记录分开存：不删数据的卸载删掉记录，密钥还在。
    pub fn secrets(&self) -> &SecretStore {
        &self.secrets
    }

    /// 插件的 Bot 记录，在记录目录下的 `bots/` 里，保留的规则同密钥。
    pub fn bots(&self) -> &BotStore {
        &self.bots
    }

    /// 插件的模型网关凭据，在记录目录下的 `model-keys/` 里，卸载时删除。
    pub fn model_keys(&self) -> &ModelKeyStore {
        &self.model_keys
    }

    /// Bot 能往哪些路径存文件，在记录目录下的 `save-grants/` 里，卸载时删除（保留数据也删）。
    pub fn save_grants(&self) -> &SaveGrantStore {
        &self.save_grants
    }

    /// 默认插件装过一次的标记，在记录目录下的 `preinstalled/` 里，卸载时保留。
    pub fn preinstalled(&self) -> &PreinstallMarks {
        &self.preinstalled
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// 磁盘上的官方索引的路径。放在子目录里：这个目录下的 `*.json` 都是插件记录。
    pub fn index_cache(&self) -> PathBuf {
        self.dir.join("store").join("official-index.json")
    }

    /// 取来的帮助文档放在这里，按提交分目录（ADR-0034）。
    pub fn docs_cache(&self) -> PathBuf {
        self.dir.join("store").join("docs")
    }

    /// 每个人的语言：用户名到最近一次设备请求的 `Accept-Language` 原文（ADR-0043）。
    pub fn people_languages(&self) -> PathBuf {
        self.dir.join("store").join(PEOPLE_LANGUAGES)
    }

    /// 读出全部记录，按 ID 排序。单个记录读不出、已损坏或与文件名不符只体现在它自己的结果里；
    /// 只有记录目录本身读不了才返回错误。文件名不是插件 ID 的文件不是核心写的，记日志后跳过。
    pub fn load_all(&self) -> Result<Vec<RecordFile>, StoreError> {
        let io = |source| StoreError::Io {
            path: self.dir.clone(),
            source,
        };
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| PluginId::parse(stem).ok());
            let Some(id) = id else {
                tracing::warn!(path = %path.display(), "插件记录目录里有无法识别的文件，已跳过");
                continue;
            };
            let record = read_record(&path, &id, &self.owner_langs());
            files.push(RecordFile { id, record });
        }
        files.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(files)
    }

    /// 一个插件的记录；没有时为 `None`。
    pub fn load(&self, id: &PluginId) -> Result<Option<InstalledRecord>, StoreError> {
        let path = self.path(id);
        if !path.exists() {
            return Ok(None);
        }
        read_record(&path, id, &self.owner_langs()).map(Some)
    }

    pub fn save(&self, record: &InstalledRecord) -> Result<(), StoreError> {
        write_json(&self.path(&record.id), record)
    }

    /// 主人的局域网设置（ADR-0019），在记录目录下的 `lan/choice.json` 里：放进子目录，不和插件记录混在一起。
    /// 出口容器只读挂载的目录：配置和两份名单。
    pub fn relay_dir(&self) -> PathBuf {
        self.dir.join("egress").join("relay")
    }

    /// 主人在 App 里存下的出网代理设置。
    pub fn egress_settings(&self) -> Result<Option<StoredEgress>, StoreError> {
        read_json(&self.dir.join("egress").join("settings.json"))
    }

    pub fn save_egress_settings(&self, settings: &StoredEgress) -> Result<(), StoreError> {
        let dir = self.dir.join("egress");
        create_private_dir(&dir)?;
        write_json(&dir.join("settings.json"), settings)
    }

    pub fn registry_mirror(
        &self,
    ) -> Result<Option<wonton_protocol::RegistryMirrorUpdate>, StoreError> {
        read_json(&self.dir.join("registry-mirror").join("settings.json"))
    }

    pub fn save_registry_mirror(
        &self,
        settings: &wonton_protocol::RegistryMirrorUpdate,
    ) -> Result<(), StoreError> {
        let dir = self.dir.join("registry-mirror");
        create_private_dir(&dir)?;
        write_json(&dir.join("settings.json"), settings)
    }

    pub fn lan_choice(&self) -> Result<Option<LanChoice>, StoreError> {
        read_json(&self.dir.join("lan").join("choice.json"))
    }

    pub fn save_lan_choice(&self, choice: &LanChoice) -> Result<(), StoreError> {
        let dir = self.dir.join("lan");
        create_private_dir(&dir)?;
        write_json(&dir.join("choice.json"), choice)
    }

    pub fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }
}

/// 只有核心能读写的目录（0700）。
pub(crate) fn create_private_dir(dir: &Path) -> Result<(), StoreError> {
    let io = |source| StoreError::Io {
        path: dir.to_owned(),
        source,
    };
    fs::create_dir_all(dir).map_err(io)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)
}

/// 以 0600 写 JSON：先写临时文件再改名，避免写到一半断电留下损坏的文件。
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    write_json_synced(path, value).map(|_| ())
}

/// 同 [`write_json`]，再告诉调用方目录项落盘了没有。要删掉记录以外的东西的调用方看这个结果。`fsync` 的错误只报告一次，别的 fd
/// 过后再 `fsync` 可能返回成功，其实没写到盘上，所以结果只能在这里拿，重试要重新写一次。
pub(crate) fn write_json_synced<T: Serialize>(path: &Path, value: &T) -> Result<bool, StoreError> {
    let tmp = path.with_extension("json.tmp");
    let io = |source| StoreError::Io {
        path: path.to_owned(),
        source,
    };
    let text = serde_json::to_string_pretty(value).map_err(|e| StoreError::Corrupt {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    file.write_all(text.as_bytes()).map_err(io)?;
    file.sync_all().map_err(io)?;
    fs::rename(&tmp, path).map_err(io)?;
    Ok(sync_parent(path))
}

/// 目录项写到盘上。有的文件系统（如网络文件系统）不支持对目录 `fsync`，那里本来也保证不了这一步。
pub(crate) fn sync_dir(dir: BorrowedFd<'_>) -> Result<(), Errno> {
    match rustix::fs::fsync(dir) {
        Ok(()) | Err(Errno::INVAL | Errno::NOTSUP) => Ok(()),
        Err(e) => Err(e),
    }
}

/// 测试用：这个目录接下来的几次 `fsync` 失败。
#[cfg(test)]
pub(crate) static DIR_SYNC_FAILURES: std::sync::Mutex<std::collections::BTreeMap<PathBuf, usize>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// 改名或删除记在目录里，断电时目录没落盘，这一步就白做了。但这一步已经生效，读的人看得到：这时报失败，调用方会以为没做成，
/// 内存和盘上就不一致了（比如再生成一个编号）。所以这里不报错，只记日志并返回有没有落盘，最坏是断电丢掉这一步。
fn sync_parent(path: &Path) -> bool {
    use std::os::fd::AsFd;
    let parent = path.parent().unwrap_or(Path::new("."));
    #[cfg(test)]
    if let Some(left) = DIR_SYNC_FAILURES.lock().unwrap().get_mut(parent)
        && *left > 0
    {
        *left -= 1;
        return false;
    }
    let synced = fs::File::open(parent)
        .map_err(|e| e.to_string())
        .and_then(|dir| sync_dir(dir.as_fd()).map_err(|e| e.to_string()));
    if let Err(error) = &synced {
        tracing::warn!(path = %parent.display(), %error, "目录落盘失败，断电可能丢掉刚才的修改");
    }
    synced.is_ok()
}

/// 读 JSON；文件不存在时返回 `None`。
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| StoreError::Corrupt {
            path: path.to_owned(),
            message: e.to_string(),
        })
}

/// 删除文件；本来就没有也算成功。
pub(crate) fn remove_file(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => {
            sync_parent(path);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// 记录每天用到的文件名。
const PEOPLE_LANGUAGES: &str = "people-languages.json";

/// 后台自己写给人看的文字（核心升级的检查和更新器，ADR-0043）按主人的语言写：那些路径上没有
/// PluginManager，按数据目录直接读。读不到时按英文，App 下一次管理请求会把它写上。
pub fn read_owner_languages(data_dir: &Path) -> Languages {
    let header = crate::owner::recorded_name(&data_dir.join("core"))
        .and_then(|owner| read_person_languages(data_dir, &owner));
    Languages::from_header(header.as_deref())
}

/// 某人最近一次请求的 `Accept-Language` 原文；没有记录时为空。
pub fn read_person_languages(data_dir: &Path, username: &str) -> Option<String> {
    read_person_languages_at(&people_languages_file(data_dir), username)
}

fn read_person_languages_at(file: &Path, username: &str) -> Option<String> {
    read_people_languages(file).get(username).cloned()
}

/// 整份“用户名 → 语言”表；文件不在时为空表，读不了或坏了时也为空表，但打一条日志，不静默。
pub fn read_people_languages(file: &Path) -> BTreeMap<String, String> {
    match read_json(file) {
        Ok(people) => people.unwrap_or_default(),
        Err(error) => {
            tracing::warn!(%error, path = %file.display(), "每个人的语言读不出来，按没有记录处理");
            BTreeMap::new()
        }
    }
}

/// 记每个人语言的目录：`<数据目录>/core/plugins/store/`。
pub fn people_languages_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("core").join("plugins").join("store")
}

fn people_languages_file(data_dir: &Path) -> PathBuf {
    people_languages_dir(data_dir).join(PEOPLE_LANGUAGES)
}

/// 名字 → 字节内容的映射在 JSON 里以 base64 保存（插件包的文件、插件密钥）。
pub(super) mod base64_map {
    use std::collections::BTreeMap;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<String, Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(map.iter().map(|(name, data)| (name, STANDARD.encode(data))))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, Vec<u8>>, D::Error> {
        BTreeMap::<String, String>::deserialize(deserializer)?
            .into_iter()
            .map(|(name, text)| {
                let data = STANDARD
                    .decode(&text)
                    .map_err(|e| D::Error::custom(format!("{name} 不是合法的 base64：{e}")))?;
                Ok((name, data))
            })
            .collect()
    }
}

impl Store {
    /// 主人的语言，读失败时按默认语言（存储层没有请求头；ADR-0043 的后台路径）。
    fn owner_langs(&self) -> Languages {
        let header = self
            .dir
            .parent()
            .and_then(crate::owner::recorded_name)
            .and_then(|owner| read_person_languages_at(&self.people_languages(), &owner));
        Languages::from_header(header.as_deref())
    }
}

fn read_record(
    path: &Path,
    id: &PluginId,
    langs: &Languages,
) -> Result<InstalledRecord, StoreError> {
    let corrupt = |message: String| StoreError::Corrupt {
        path: path.to_owned(),
        message,
    };
    let text = fs::read_to_string(path).map_err(|source| StoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    let record: InstalledRecord =
        serde_json::from_str(&text).map_err(|e| corrupt(e.to_string()))?;
    if &record.id != id {
        return Err(corrupt(crate::l10n::text(
            langs,
            "plugin-settings-store-record-id-mismatch",
            &[
                ("record", record.id.to_string().into()),
                ("file", id.to_string().into()),
            ],
        )));
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> InstalledRecord {
        InstalledRecord {
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            id: PluginId::parse(id).unwrap(),
            version: "1.0.0".into(),
            source: "services: {}".into(),
            files: [("config.yaml".to_owned(), b"a: \x01\xff".to_vec())].into(),
            granted_permissions: vec!["storage.share.photos:read".parse().unwrap()],
            enabled_permissions: Vec::new(),
            limits: Default::default(),
            oidc_client_secret: Some("s3cret".into()),
            installed_at: 1,
            origin: None,
            written_by: None,
            ask_every_update: false,
            family: false,
        }
    }

    fn readable(store: &Store) -> Vec<InstalledRecord> {
        let files = store.load_all().unwrap();
        files.into_iter().map(|f| f.record.unwrap()).collect()
    }

    #[test]
    fn records_survive_reopen_and_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("plugins")).unwrap();
        let record = record("app.test");
        store.save(&record).unwrap();
        let reopened = Store::open(dir.path().join("plugins")).unwrap();
        assert_eq!(readable(&reopened), vec![record.clone()]);
        let mode = fs::metadata(reopened.path(&record.id))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        reopened.remove(&record.id).unwrap();
        reopened.remove(&record.id).unwrap();
        assert!(reopened.load_all().unwrap().is_empty());
    }

    #[test]
    fn a_broken_record_is_reported_under_its_file_name_without_hiding_others() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let good = record("app.good");
        store.save(&good).unwrap();
        fs::write(dir.path().join("app.broken.json"), "{\"id\": ").unwrap();
        // 记录里的 ID 和文件名不符：按文件名报告，不能冒充另一个插件，否则卸载时删不掉这个文件。
        let other = serde_json::to_string(&record("app.other")).unwrap();
        fs::write(dir.path().join("app.renamed.json"), other).unwrap();
        // 文件名不是插件 ID：不是核心写的，跳过。
        fs::write(dir.path().join("notes.json"), "{}").unwrap();
        let files = store.load_all().unwrap();
        let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["app.broken", "app.good", "app.renamed"]);
        assert!(matches!(files[0].record, Err(StoreError::Corrupt { .. })));
        assert_eq!(files[1].record.as_ref().unwrap(), &good);
        assert!(matches!(files[2].record, Err(StoreError::Corrupt { .. })));
    }
}
