//! Passwords used by third-party HTTP clients (ADR-0021).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

use crate::secret::constant_time_eq;

/// 存放位置（相对核心数据目录）：0600 的 JSON，明文值主人能在 App 里看（ADR-0021「在 App 里」）。
pub(crate) const FILE_NAME: &str = "clients/passwords.json";

/// `per_client: false` 的唯一一条密码的记录 ID。
pub(crate) const SHARED_ID: &str = "shared";

/// 没经设备取值（主人令牌）时记的“设备”。
pub(crate) const OWNER_TOKEN: &str = "owner_token";

const TOKEN_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// 一条密码在应用里的样子：已经换好，或者还有吊销/换值没完成。
///
/// 未完成时值照旧可用（[`PasswordStore::authenticate`] 不跳过它）：命令可能已经在应用里执行，
/// 这时当已吊销是错的。完成前重跑同一个操作，别的操作都不碰。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum State {
    /// 应用里就是 `value`。
    #[default]
    Ready,
    /// 换了值还没执行完；`next` 是核心生成的新值，`None` 时由下一次 `rotate` 从标准输出生成。
    PendingRotate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
    },
    /// 吊销还没执行完；值还留着，重试成功才删。
    PendingRevoke,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    label: String,
    value: String,
    /// 建它的设备名，App 里显示用；不知道时为空。
    #[serde(default)]
    created_by_device: Option<String>,
    /// 建它的设备 ID；设备吊销按它连带（ADR-0046）。
    #[serde(default)]
    created_by: Option<String>,
    /// 看过它的设备 ID → 最近一次取值时间（Unix 毫秒）。
    #[serde(default)]
    viewed_by: BTreeMap<String, u64>,
    #[serde(default)]
    state: State,
    /// C16 的旧写法，读旧文件用；`true` 读回成 [`State::PendingRevoke`]，写出去不再带。
    #[serde(default, skip_serializing)]
    pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    plugins: BTreeMap<String, Vec<Record>>,
    /// 成功发过一次密码的插件：入口在这些插件上才打得开（ADR-0021「入口的开关」）。
    #[serde(default)]
    issued: std::collections::BTreeSet<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("read or write client password storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("client password storage is corrupt: {0}")]
    Corrupt(String),
    #[error("client password was not found")]
    NotFound,
    #[error("client password label is invalid")]
    InvalidLabel,
}

/// （插件，密码 ID）→ 吊销信号：吊销时置位，转发层据此断开已升级的连接（ADR-0021「吊销」）。
type Guards = Arc<Mutex<BTreeMap<(String, String), watch::Sender<bool>>>>;

/// 吊销不结束登录的应用留下的信号：发送者活着，已升级的连接就不会看到关闭。
type Lingering = Arc<Mutex<Vec<watch::Sender<bool>>>>;

#[derive(Clone)]
pub struct PasswordStore {
    path: PathBuf,
    state: Arc<Mutex<File>>,
    guards: Guards,
    lingering: Lingering,
}

#[derive(Debug, Clone)]
pub struct Created {
    pub id: String,
    pub value: String,
}

/// 一条密码的只读快照；状态机在锁外按它决定跑哪个命令。
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub id: String,
    /// 只在测试里用来挑记录。
    #[cfg_attr(not(test), allow(dead_code))]
    pub label: String,
    pub value: String,
    pub state: State,
}

impl PasswordStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, PasswordError> {
        let path = data_dir.as_ref().join(FILE_NAME);
        let mut state: File = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|error| PasswordError::Corrupt(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(error) => return Err(error.into()),
        };
        // C16 的 `pending: true` 是“吊销没完成”：读成待吊销，别当成已经落地。
        for records in state.plugins.values_mut() {
            for record in records.iter_mut() {
                if std::mem::take(&mut record.pending) && record.state == State::Ready {
                    record.state = State::PendingRevoke;
                }
            }
        }
        let store = Self {
            path,
            state: Arc::new(Mutex::new(state)),
            guards: Arc::new(Mutex::new(BTreeMap::new())),
            lingering: Arc::new(Mutex::new(Vec::new())),
        };
        store.ensure_mode()?;
        Ok(store)
    }

    /// `device` 是（设备 ID，设备名）：设备吊销按 ID 连带（ADR-0046），界面显示用名字。
    /// 已落地：网关验证的密码直接就能用（C16）。
    pub fn create(
        &self,
        plugin: &str,
        label: &str,
        device: Option<(&str, Option<&str>)>,
    ) -> Result<Created, PasswordError> {
        self.insert(plugin, label, device, None, State::Ready)
    }

    /// 核心生成值的待换：先落盘，命令成功才转正（C17）。
    pub(crate) fn create_pending(
        &self,
        plugin: &str,
        label: &str,
        device: Option<(&str, Option<&str>)>,
        value: String,
    ) -> Result<Created, PasswordError> {
        let created = self.insert(
            plugin,
            label,
            device,
            Some(value.clone()),
            State::PendingRotate { next: Some(value) },
        )?;
        Ok(created)
    }

    /// `generates` 的待换：值由应用自己出，成功前没有。
    pub(crate) fn create_generated_pending(
        &self,
        plugin: &str,
        label: &str,
        device: Option<(&str, Option<&str>)>,
    ) -> Result<Created, PasswordError> {
        self.insert(
            plugin,
            label,
            device,
            Some(String::new()),
            State::PendingRotate { next: None },
        )
    }

    fn insert(
        &self,
        plugin: &str,
        label: &str,
        device: Option<(&str, Option<&str>)>,
        value: Option<String>,
        state: State,
    ) -> Result<Created, PasswordError> {
        if label.trim().is_empty() || label.len() > 128 || label.contains(['\n', '\r']) {
            return Err(PasswordError::InvalidLabel);
        }
        let mut state_file = self.state.lock().expect("client password lock poisoned");
        let records = state_file.plugins.entry(plugin.to_owned()).or_default();
        let id = next_id(records);
        self.push_locked(plugin, records, id, label, device, value, state)?;
        self.save_locked(&state_file)?;
        let records = state_file.plugins.get(plugin).expect("刚插入的记录");
        let record = records.last().expect("刚插入的记录");
        Ok(Created {
            id: record.id.clone(),
            value: record.value.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn push_locked(
        &self,
        plugin: &str,
        records: &mut Vec<Record>,
        id: String,
        label: &str,
        device: Option<(&str, Option<&str>)>,
        value: Option<String>,
        state: State,
    ) -> Result<(), PasswordError> {
        let value = value.unwrap_or_else(random_base32);
        records.push(Record {
            id: id.clone(),
            label: label.to_owned(),
            value,
            created_by_device: device.and_then(|(_, name)| name).map(str::to_owned),
            created_by: device.map(|(id, _)| id.to_owned()),
            viewed_by: BTreeMap::new(),
            state,
            pending: false,
        });
        self.guards
            .lock()
            .expect("client password guard lock poisoned")
            .insert((plugin.to_owned(), id), watch::channel(false).0);
        Ok(())
    }

    /// `per_client: false` 的唯一一条密码。已经有了时不新建（返回 `None`）。
    pub(crate) fn ensure_shared(
        &self,
        plugin: &str,
        label: &str,
        device: Option<(&str, Option<&str>)>,
        value: Option<String>,
        next: Option<String>,
    ) -> Result<Option<Created>, PasswordError> {
        if label.trim().is_empty() || label.len() > 128 || label.contains(['\n', '\r']) {
            return Err(PasswordError::InvalidLabel);
        }
        let mut state = self.state.lock().expect("client password lock poisoned");
        let records = state.plugins.entry(plugin.to_owned()).or_default();
        if records.iter().any(|record| record.id == SHARED_ID) {
            return Ok(None);
        }
        let state_value = State::PendingRotate { next: next.clone() };
        self.push_locked(
            plugin,
            records,
            SHARED_ID.to_owned(),
            label,
            device,
            value,
            state_value,
        )?;
        let value = records
            .iter()
            .find(|record| record.id == SHARED_ID)
            .map(|record| record.value.clone())
            .expect("刚插入共享密码");
        self.save_locked(&state)?;
        Ok(Some(Created {
            id: SHARED_ID.to_owned(),
            value,
        }))
    }

    pub fn read(
        &self,
        plugin: &str,
        id: &str,
        device: Option<&str>,
    ) -> Result<String, PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let record = state
            .plugins
            .get_mut(plugin)
            .and_then(|records| records.iter_mut().find(|record| record.id == id))
            .ok_or(PasswordError::NotFound)?;
        record
            .viewed_by
            .insert(device.unwrap_or(OWNER_TOKEN).to_owned(), now_ms());
        let value = record.value.clone();
        self.save_locked(&state)?;
        Ok(value)
    }

    pub(crate) fn summaries(
        &self,
        plugin: &str,
        device_name: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<wonton_protocol::PasswordSummary>, PasswordError> {
        let state = self
            .state
            .lock()
            .map_err(|error| PasswordError::Corrupt(error.to_string()))?;
        // Vec 的追加顺序就是创建顺序；删除后复用的 ID 不能拿来排序。
        Ok(state
            .plugins
            .get(plugin)
            .into_iter()
            .flatten()
            .map(|record| {
                let mut viewed: Vec<_> = record.viewed_by.iter().collect();
                viewed
                    .sort_by(|(a, time_a), (b, time_b)| time_b.cmp(time_a).then_with(|| a.cmp(b)));
                wonton_protocol::PasswordSummary {
                    id: record.id.clone(),
                    label: record.label.clone(),
                    created_by_device: record.created_by_device.clone(),
                    last_viewed_by: viewed
                        .into_iter()
                        .filter_map(|(id, _)| device_name(id))
                        .collect(),
                    pending: match record.state {
                        State::Ready => None,
                        State::PendingRevoke => Some(wonton_protocol::PendingOp::Revoke),
                        State::PendingRotate { .. } => Some(wonton_protocol::PendingOp::Rotate),
                    },
                }
            })
            .collect())
    }

    /// 这条密码（不论待换还是待吊销）的当前值是否是这个；待换时旧值仍算数。
    pub fn authenticate(
        &self,
        plugin: &str,
        supplied: &str,
    ) -> Result<Option<watch::Receiver<bool>>, PasswordError> {
        let state = self.state.lock().expect("client password lock poisoned");
        let Some(records) = state.plugins.get(plugin) else {
            return Ok(None);
        };
        let Some(record) = records
            .iter()
            .find(|record| constant_time_eq(&record.value, supplied))
        else {
            return Ok(None);
        };
        let key = (plugin.to_owned(), record.id.clone());
        let mut guards = self
            .guards
            .lock()
            .expect("client password guard lock poisoned");
        Ok(Some(
            guards
                .entry(key)
                .or_insert_with(|| watch::channel(false).0)
                .subscribe(),
        ))
    }

    /// 值命中的那条记录和它的吊销信号（`credentials` 要在应用里验证，得知道记录 ID）。
    pub(crate) fn match_password(
        &self,
        plugin: &str,
        supplied: &str,
    ) -> Result<Option<(Snapshot, watch::Receiver<bool>)>, PasswordError> {
        let state = self.state.lock().expect("client password lock poisoned");
        let Some(records) = state.plugins.get(plugin) else {
            return Ok(None);
        };
        let Some(record) = records
            .iter()
            .find(|record| constant_time_eq(&record.value, supplied))
        else {
            return Ok(None);
        };
        let snapshot = record.snapshot();
        let key = (plugin.to_owned(), record.id.clone());
        drop(state);
        let mut guards = self
            .guards
            .lock()
            .expect("client password guard lock poisoned");
        let revoke = guards
            .entry(key)
            .or_insert_with(|| watch::channel(false).0)
            .subscribe();
        Ok(Some((snapshot, revoke)))
    }

    pub(crate) fn snapshot(&self, plugin: &str, id: &str) -> Result<Snapshot, PasswordError> {
        let state = self.state.lock().expect("client password lock poisoned");
        let record = find(&state, plugin, id)?;
        Ok(record.snapshot())
    }

    pub(crate) fn snapshots(&self, plugin: &str) -> Vec<Snapshot> {
        let state = self.state.lock().expect("client password lock poisoned");
        state
            .plugins
            .get(plugin)
            .map(|records| records.iter().map(|record| record.snapshot()).collect())
            .unwrap_or_default()
    }

    /// 这台设备建过或看过的密码，按插件分组。
    pub(crate) fn for_device(&self, device: &str) -> Vec<(String, String)> {
        let state = self.state.lock().expect("client password lock poisoned");
        let mut touched = Vec::new();
        for (plugin, records) in &state.plugins {
            for record in records {
                if record.created_by.as_deref() == Some(device)
                    || record.viewed_by.contains_key(device)
                {
                    touched.push((plugin.clone(), record.id.clone()));
                }
            }
        }
        touched
    }

    /// 落盘“待吊销”；成功后由 [`Self::finish_revoke`] 删除。
    pub(crate) fn begin_revoke(&self, plugin: &str, id: &str) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let record = find_mut(&mut state, plugin, id)?;
        if record.state == State::PendingRevoke {
            return Ok(());
        }
        record.state = State::PendingRevoke;
        self.save_locked(&state)
    }

    /// 落盘“待换值”；`next` 为 `None` 时下次 `rotate` 自己生成。
    pub(crate) fn begin_rotate(
        &self,
        plugin: &str,
        id: &str,
        next: Option<String>,
    ) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let record = find_mut(&mut state, plugin, id)?;
        record.state = State::PendingRotate { next };
        self.save_locked(&state)
    }

    /// 吊销执行成功：先置守卫断开已升级的连接，再删记录。
    pub(crate) fn finish_revoke(
        &self,
        plugin: &str,
        id: &str,
        ends_sessions: bool,
    ) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let records = state
            .plugins
            .get_mut(plugin)
            .ok_or(PasswordError::NotFound)?;
        let index = records
            .iter()
            .position(|record| record.id == id)
            .ok_or(PasswordError::NotFound)?;
        records.remove(index);
        self.save_locked(&state)?;
        let sender = self
            .guards
            .lock()
            .expect("client password guard lock poisoned")
            .remove(&(plugin.to_owned(), id.to_owned()));
        if let Some(sender) = sender {
            if ends_sessions {
                sender.send_replace(true);
            } else {
                // 应用自己维持登录：留着信号，已升级的连接不断。连接早就断了的不留。
                let mut lingering = self
                    .lingering
                    .lock()
                    .expect("client password guard lock poisoned");
                lingering.retain(|sender| sender.receiver_count() > 0);
                lingering.push(sender);
            }
        }
        Ok(())
    }

    /// 换值执行成功：待换的新值生效（应用自己生成时用 `generated`），状态回到已落地。
    /// `ends_sessions` 时置守卫断开已升级的连接；新连接用新值换一个新守卫。
    pub(crate) fn finish_rotate(
        &self,
        plugin: &str,
        id: &str,
        generated: Option<String>,
        ends_sessions: bool,
    ) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let record = find_mut(&mut state, plugin, id)?;
        let State::PendingRotate { next } = &record.state else {
            return Ok(());
        };
        if let Some(value) = next.clone().or(generated) {
            record.value = value;
        }
        record.state = State::Ready;
        self.save_locked(&state)?;
        if ends_sessions {
            self.end_sessions(plugin, id);
        }
        Ok(())
    }

    /// 断掉这条密码已经升级的连接，下一个请求重新建守卫。
    fn end_sessions(&self, plugin: &str, id: &str) {
        let sender = self
            .guards
            .lock()
            .expect("client password guard lock poisoned")
            .remove(&(plugin.to_owned(), id.to_owned()));
        if let Some(sender) = sender {
            sender.send_replace(true);
        }
    }
    /// 网关验证的密码没有命令：一步落盘、置守卫、删除。
    pub fn revoke(&self, plugin: &str, id: &str) -> Result<(), PasswordError> {
        self.begin_revoke(plugin, id)?;
        self.finish_revoke(plugin, id, true)
    }

    pub fn remove_plugin(&self, plugin: &str) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        state.issued.remove(plugin);
        if let Some(records) = state.plugins.remove(plugin) {
            let guards = self
                .guards
                .lock()
                .expect("client password guard lock poisoned");
            for record in records {
                if let Some(sender) = guards.get(&(plugin.to_owned(), record.id)) {
                    sender.send_replace(true);
                }
            }
        }
        self.save_locked(&state)
    }

    /// 这个插件成功发过一次密码没有。
    pub(crate) fn issued(&self, plugin: &str) -> bool {
        self.state
            .lock()
            .expect("client password lock poisoned")
            .issued
            .contains(plugin)
    }

    pub(crate) fn mark_issued(&self, plugin: &str) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        if state.issued.insert(plugin.to_owned()) {
            self.save_locked(&state)?;
        }
        Ok(())
    }

    fn save_locked(&self, state: &File) -> Result<(), PasswordError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| PasswordError::Corrupt("password path has no parent".to_owned()))?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(state)
            .map_err(|error| PasswordError::Corrupt(error.to_string()))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;
        self.ensure_mode()
    }

    fn ensure_mode(&self) -> Result<(), PasswordError> {
        if self.path.exists() {
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

impl Record {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            id: self.id.clone(),
            label: self.label.clone(),
            value: self.value.clone(),
            state: self.state.clone(),
        }
    }
}

fn find<'a>(state: &'a File, plugin: &str, id: &str) -> Result<&'a Record, PasswordError> {
    state
        .plugins
        .get(plugin)
        .and_then(|records| records.iter().find(|record| record.id == id))
        .ok_or(PasswordError::NotFound)
}

fn find_mut<'a>(
    state: &'a mut File,
    plugin: &str,
    id: &str,
) -> Result<&'a mut Record, PasswordError> {
    state
        .plugins
        .get_mut(plugin)
        .and_then(|records| records.iter_mut().find(|record| record.id == id))
        .ok_or(PasswordError::NotFound)
}

fn next_id(records: &[Record]) -> String {
    let mut number = records.len() + 1;
    loop {
        let id = format!("pw-{number}");
        if !records.iter().any(|record| record.id == id) {
            return id;
        }
        number += 1;
    }
}

/// 新密码值：128 位随机数的小写 base32。
pub(crate) fn random_value() -> String {
    random_base32()
}

fn random_base32() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS randomness unavailable");
    let mut output = String::with_capacity(26);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(TOKEN_ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits != 0 {
        output.push(TOKEN_ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    output
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn stored_0600() {
        let dir = tempfile::tempdir().unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        let created = store
            .create("app.demo", "phone", Some(("device-a", Some("iPad"))))
            .unwrap();
        assert_eq!(created.value.len(), 26);
        assert!(
            created
                .value
                .bytes()
                .all(|byte| TOKEN_ALPHABET.contains(&byte))
        );
        assert_eq!(
            fs::metadata(dir.path().join(FILE_NAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn revocation_invalidates_authentication_and_stream_guard() {
        let dir = tempfile::tempdir().unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        let created = store.create("app.demo", "phone", None).unwrap();
        let receiver = store
            .authenticate("app.demo", &created.value)
            .unwrap()
            .unwrap();
        store.revoke("app.demo", "pw-1").unwrap();
        assert!(*receiver.borrow());
        assert!(
            store
                .authenticate("app.demo", &created.value)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn pending_value_stays_usable_until_finished() {
        let dir = tempfile::tempdir().unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        let created = store.create("app.demo", "phone", None).unwrap();
        store
            .begin_rotate("app.demo", &created.id, Some("next".into()))
            .unwrap();
        assert_eq!(
            store.snapshot("app.demo", &created.id).unwrap().state,
            State::PendingRotate {
                next: Some("next".into())
            }
        );
        assert!(
            store
                .authenticate("app.demo", &created.value)
                .unwrap()
                .is_some()
        );
        store
            .finish_rotate("app.demo", &created.id, None, false)
            .unwrap();
        assert_eq!(
            store.snapshot("app.demo", &created.id).unwrap().value,
            "next"
        );
    }

    #[test]
    fn legacy_pending_reads_as_pending_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"plugins":{"app.demo":[{"id":"pw-1","label":"phone","value":"v","pending":true}]}}"#,
        )
        .unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        assert_eq!(
            store.snapshot("app.demo", "pw-1").unwrap().state,
            State::PendingRevoke
        );
        // 迁移在内存里完成；下一次落盘（取值会记 viewed_by）写出去才不带旧键。
        store.read("app.demo", "pw-1", None).unwrap();
        let reopened = PasswordStore::open(dir.path()).unwrap();
        assert!(
            !fs::read_to_string(dir.path().join(FILE_NAME))
                .unwrap()
                .contains("\"pending\"")
        );
        assert_eq!(
            reopened.snapshot("app.demo", "pw-1").unwrap().state,
            State::PendingRevoke
        );
    }

    #[test]
    fn issue_flag_survives_reopen_and_plugin_removal_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        assert!(!store.issued("app.demo"));
        store.mark_issued("app.demo").unwrap();
        assert!(PasswordStore::open(dir.path()).unwrap().issued("app.demo"));
        store.remove_plugin("app.demo").unwrap();
        assert!(!PasswordStore::open(dir.path()).unwrap().issued("app.demo"));
    }
}
