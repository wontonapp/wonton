//! 成员和加入请求（ADR-0088）：记录在 `core/members.json`，整份重写，0600，启动时读回。
//! 已移除的成员留在表里，用户名不再分配。邀请码只存 SHA-256；请求绑定设备 endpoint，批准后的同设备重试到期前有效。

mod join_requests;
mod lifecycle;

pub(crate) use lifecycle::Lifecycle;

pub use join_requests::JoinCheck;

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wonton_protocol::{
    MEMBER_PAIRING_CODE_TTL_SECS, MemberStatus, OwnerProfileUpdate, ProfileProblem,
    check_display_name, check_email_change, normalized_email,
};

use wonton_tunnel::pair::{PairingSecret, SECRET_LEN};

use crate::owner;

/// 记录里没有角色时算“朋友”：权限最少，不冒充家人或管理员。
fn default_role_id() -> String {
    crate::roles::DEFAULT_INVITE_ROLE.to_owned()
}

fn invalid(problem: ProfileProblem) -> MembersError {
    MembersError::Invalid(problem.to_string())
}

/// 码的密钥记成 SHA-256：`members.json` 里的码不能反推出来。
fn code_hash(secret_hex: &str) -> String {
    let digest = Sha256::digest(secret_hex.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[derive(Debug, thiserror::Error)]
pub enum MembersError {
    #[error("读写家人记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("家人记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("没有这个家人：{0}")]
    NotFound(String),
    #[error("{0} 已被移除：移除后的家人不能出码")]
    Removed(String),
    #[error(transparent)]
    Device(#[from] crate::device::DeviceError),
    #[error("这台设备已经登记，不能改变归属")]
    DeviceRegistered,
}

/// 成员变化通知。收尾根据持久记录重算，不依赖事件可靠送达。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberEvent {
    Joined {
        username: String,
    },
    Removed {
        username: String,
        delete_home: bool,
        keep_bot_data: bool,
    },
}

/// 一个家人，存进 `members.json` 的写法。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub username: String,
    /// Matrix 用户 ID，邀请时按当时的 `server_name` 拼好。
    pub user_id: String,
    /// 邀请时主人填的，家人加入后由他自己改。
    pub display_name: String,
    /// 家人自己填；没填时没有。填过以后只能改，不能清空（规则同 ADR-0030）。
    #[serde(default)]
    pub email: Option<String>,
    pub status: MemberStatus,
    /// 他的角色（`roles.json` 的 ID）。记录里没有这一项时按权限最少的默认角色算。
    #[serde(default = "default_role_id")]
    pub role_id: String,
    /// 邀请时选择创建私人 Bot；角色也必须允许。
    #[serde(default)]
    pub wants_private_bot: bool,
    /// 他的私人 Bot；还没建时为空。
    #[serde(default)]
    pub private_bot: Option<wonton_protocol::BotRef>,
    /// 还有效的配对码：密钥的 SHA-256（十六进制小写）和过期时间（Unix 毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_code_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_code_expires_at_ms: Option<u64>,
}

impl MemberRecord {
    /// 测试里记一个家人：用户名、显示名和用户 ID 由用户名派生，状态“已邀请”，其余按最小。
    /// 只有测试用，生产走邀请。
    #[cfg(test)]
    pub fn for_test(username: &str) -> Self {
        Self {
            username: username.to_owned(),
            user_id: format!("@{username}:test"),
            display_name: username.to_owned(),
            email: None,
            status: MemberStatus::Invited,
            role_id: default_role_id(),
            wants_private_bot: true,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Stored {
    members: Vec<MemberRecord>,
    #[serde(default)]
    joins: Vec<join_requests::StoredJoin>,
    #[serde(default)]
    lifecycle: std::collections::BTreeMap<String, Lifecycle>,
}

/// 发出去的归家人的码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCode {
    pub secret: PairingSecret,
    /// 什么时候过期（Unix 毫秒）。
    pub expires_at_ms: u64,
}

/// 家人记录。整份重写、随数据目录备份恢复；用户名固定、不复用（ADR-0022）。
pub struct Members {
    path: PathBuf,
    members: Mutex<Stored>,
    events: tokio::sync::broadcast::Sender<MemberEvent>,
    now_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for Members {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Members")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Members {
    /// 读取记录；文件不存在时从空表开始。已损坏时启动失败，不覆盖。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, MembersError> {
        Self::with_clock(path, Arc::new(system_ms))
    }

    /// 测试用：注入时钟，不 sleep 就能测过期。
    pub fn with_clock(
        path: impl Into<PathBuf>,
        now_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, MembersError> {
        let path = path.into();
        let members = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| MembersError::Corrupt {
                path: path.clone(),
                message: e.to_string(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(source) => {
                return Err(MembersError::Io { path, source });
            }
        };
        // 广播通道的容量给排队留余地：后续任务的处理慢时丢的是最老的事件，Joined 和 Removed 都会重扫全家。
        let (events, _) = tokio::sync::broadcast::channel(64);
        Ok(Self {
            path,
            members: Mutex::new(members),
            events,
            now_ms,
        })
    }

    /// 家人事件的订阅端。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<MemberEvent> {
        self.events.subscribe()
    }

    /// 全部家人，包括已移除的；按用户名排序。
    pub fn list(&self) -> Vec<MemberRecord> {
        let mut members = self.lock().members.clone();
        members.sort_by(|a, b| a.username.cmp(&b.username));
        members
    }

    pub fn get(&self, username: &str) -> Option<MemberRecord> {
        self.lock()
            .members
            .iter()
            .find(|m| m.username == username)
            .cloned()
    }

    /// 这个用户名是不是任何一条家人记录（包括已移除的）。
    pub fn taken(&self, username: &str) -> bool {
        self.lock().members.iter().any(|m| m.username == username)
    }

    pub(crate) fn count_role(&self, role_id: &str) -> usize {
        self.lock()
            .members
            .iter()
            .filter(|member| member.status != MemberStatus::Removed && member.role_id == role_id)
            .count()
    }

    /// 记下一个“已邀请”的家人。用户名的查重由调用方做完，这里只拦记录里已经有的。
    pub fn invite(&self, record: MemberRecord) -> Result<MemberRecord, MembersError> {
        let mut members = self.lock();
        if members
            .members
            .iter()
            .any(|m| m.username == record.username)
        {
            return Err(MembersError::NotFound(record.username));
        }
        let record = MemberRecord {
            status: MemberStatus::Invited,
            ..record
        };
        self.save(&mut members, record.clone())?;
        Ok(record)
    }

    /// 主人给别人换角色（`roles.json` 的 ID）。角色在不在由调用方先查。
    pub fn set_role(&self, username: &str, role_id: &str) -> Result<MemberRecord, MembersError> {
        let mut stored = self.lock();
        let mut next = stored.clone();
        let record = next
            .members
            .iter_mut()
            .find(|member| member.username == username)
            .ok_or_else(|| MembersError::NotFound(username.to_owned()))?;
        if record.status == MemberStatus::Removed {
            return Err(MembersError::Removed(username.to_owned()));
        }
        record.role_id = role_id.to_owned();
        let record = record.clone();
        for join in &mut next.joins {
            if join.request.username == username {
                join.request.role_id = role_id.to_owned();
            }
        }
        self.commit(&mut stored, next)?;
        Ok(record)
    }

    /// 家人改自己的资料，规则同 ADR-0030（[`owner`] 的检查）：名字必填，邮箱填过以后只能改，不能清空。
    /// 已移除的家人不能再改：记录留着只是为了占住用户名。
    pub fn update_profile(
        &self,
        username: &str,
        update: &OwnerProfileUpdate,
    ) -> Result<MemberRecord, MembersError> {
        let previous = self
            .get(username)
            .ok_or_else(|| MembersError::NotFound(username.to_owned()))?;
        if previous.status == MemberStatus::Removed {
            return Err(MembersError::Removed(username.to_owned()));
        }
        let display_name = update.display_name.trim();
        let email = normalized_email(update.email.as_deref());
        check_display_name(display_name).map_err(invalid)?;
        check_email_change(previous.email.as_deref(), email).map_err(invalid)?;
        self.update(username, |record| {
            // 改动和查重在同一把锁里：写盘前别人的修改进不来。
            check_email_change(record.email.as_deref(), email).map_err(invalid)?;
            record.display_name = display_name.to_owned();
            record.email = email.map(str::to_owned);
            Ok(())
        })
    }

    /// 主人给这个家人出码：一次一个，再出一个旧的作废。已移除的家人不再出码。
    pub fn issue_pairing_code(&self, username: &str) -> Result<IssuedCode, MembersError> {
        let mut stored = self.lock();
        let mut next = stored.clone();
        let record = next
            .members
            .iter_mut()
            .find(|m| m.username == username)
            .ok_or_else(|| MembersError::NotFound(username.to_owned()))?;
        if record.status == MemberStatus::Removed {
            return Err(MembersError::Removed(username.to_owned()));
        }
        let mut bytes = [0u8; SECRET_LEN];
        getrandom::fill(&mut bytes).expect("系统随机数不可用");
        let secret = PairingSecret::from_bytes(bytes);
        let expires_at_ms = (self.now_ms)() + MEMBER_PAIRING_CODE_TTL_SECS * 1000;
        record.pairing_code_hash = Some(code_hash(secret.expose()));
        record.pairing_code_expires_at_ms = Some(expires_at_ms);
        next.joins.retain(|join| join.request.username != username);
        self.commit(&mut stored, next)?;
        Ok(IssuedCode {
            secret,
            expires_at_ms,
        })
    }

    /// 作废这个家人的码，不出新的。
    pub fn void_pairing_code(&self, username: &str) -> Result<(), MembersError> {
        self.update(username, |record| {
            record.pairing_code_hash = None;
            record.pairing_code_expires_at_ms = None;
            Ok(())
        })?;
        Ok(())
    }

    /// 测试夹具跳过真实配对；生产只能批准加入请求。
    #[cfg(test)]
    pub(crate) fn mark_joined(&self, username: &str) -> Result<bool, MembersError> {
        let mut joined = false;
        self.update(username, |record| {
            if record.status == MemberStatus::Invited {
                record.status = MemberStatus::Joined;
                joined = true;
            }
            Ok(())
        })?;
        if joined {
            tracing::info!(member = %username, "家人已加入");
            if self
                .events
                .send(MemberEvent::Joined {
                    username: username.to_owned(),
                })
                .is_err()
            {
                tracing::debug!(member = %username, "Joined 事件还没有订阅者");
            }
        }
        Ok(joined)
    }

    /// 移除家人：状态改成“已移除”，作废他的码，发 [`MemberEvent::Removed`]。吊销设备由调用方做
    /// （要走 [`crate::device::Devices`]，隧道连接立即断开）。
    pub fn remove(
        &self,
        username: &str,
        delete_home: bool,
        keep_bot_data: bool,
    ) -> Result<MemberRecord, MembersError> {
        let mut stored = self.lock();
        let mut next = stored.clone();
        let member = next
            .members
            .iter_mut()
            .find(|member| member.username == username)
            .ok_or_else(|| MembersError::NotFound(username.to_owned()))?;
        member.status = MemberStatus::Removed;
        // 保留哈希才能向持旧码的设备说明成员已移除；保留收尾选项才能在失败或重启后继续。
        let record = member.clone();
        next.lifecycle
            .entry(username.to_owned())
            .or_default()
            .removal = Some(wonton_protocol::MemberRemoval {
            delete_home,
            keep_bot_data,
        });
        self.commit(&mut stored, next)?;
        drop(stored);
        tracing::info!(member = %username, "家人已移除");
        let _ = self.events.send(MemberEvent::Removed {
            username: username.to_owned(),
            delete_home,
            keep_bot_data,
        });
        Ok(record)
    }

    fn update(
        &self,
        username: &str,
        change: impl FnOnce(&mut MemberRecord) -> Result<(), MembersError>,
    ) -> Result<MemberRecord, MembersError> {
        let mut members = self.lock();
        let mut next = members.clone();
        let Some(record) = next.members.iter_mut().find(|m| m.username == username) else {
            return Err(MembersError::NotFound(username.to_owned()));
        };
        change(record)?;
        // 先复制要返回的记录，可变借用就此结束，整份快照才能克隆出来。
        let updated = next
            .members
            .iter()
            .find(|m| m.username == username)
            .cloned()
            .expect("update 找到了这条记录");
        self.commit(&mut members, next)?;
        Ok(updated)
    }

    fn save(&self, members: &mut Stored, record: MemberRecord) -> Result<(), MembersError> {
        let mut next = members.clone();
        next.members.push(record);
        self.commit(members, next)
    }

    fn commit(&self, stored: &mut Stored, next: Stored) -> Result<(), MembersError> {
        write_record(&self.path, &next)?;
        *stored = next;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Stored> {
        self.members.lock().expect("家人记录锁中毒")
    }
}

fn system_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// 先写临时文件再改名：写到一半掉电时，下次启动不会读到半个记录。文件权限 0600：记录里没有码的原文，
/// 但家人名单是隐私。
fn write_record(path: &std::path::Path, stored: &Stored) -> Result<(), MembersError> {
    let io = |source| MembersError::Io {
        path: path.to_owned(),
        source,
    };
    let text = serde_json::to_string_pretty(stored).map_err(|e| MembersError::Corrupt {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    fs::rename(&tmp, path).map_err(io)?;
    fs::File::open(path)
        .and_then(|file| file.set_permissions(fs::Permissions::from_mode(0o600)))
        .map_err(io)?;
    Ok(())
}

/// 邀请时按显示名生成用户名：显示名是 ASCII 的才生成——转小写、去掉不合 ADR-0022 规则的字符；
/// 不是 ASCII、或者清理后不合规则（空了、开头不是字母），返回没有，调用方用 `family<n>`。
pub fn username_candidate(display_name: &str) -> Option<String> {
    if !display_name.is_ascii() {
        return None;
    }
    let cleaned: String = display_name
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .collect();
    owner::check_username(&cleaned).ok().map(|_| cleaned)
}

/// `family<n>`，n 从 1 起取最小的空号。
pub fn family_username(taken: impl Fn(&str) -> bool) -> String {
    (1..)
        .map(|n| format!("family{n}"))
        .find(|candidate| !taken(candidate))
        .expect("家人数量不设上限，空号总能找到")
}

#[cfg(test)]
#[path = "members_join_tests.rs"]
mod members_join_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::MemberInvite;

    #[test]
    fn invite_survives_a_restart_and_removed_usernames_are_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("members.json");
        let members = Members::open(&path).unwrap();
        members.invite(MemberRecord::for_test("alice")).unwrap();
        members.set_role("alice", "admin").unwrap();
        members.remove("alice", false, true).unwrap();

        let reopened = Members::open(&path).unwrap();
        let alice = reopened.get("alice").unwrap();
        assert_eq!(alice.status, MemberStatus::Removed);
        assert_eq!(alice.role_id, "admin", "角色跟着记录走");
        assert!(reopened.taken("alice"), "移除后用户名不再分配");
        assert!(matches!(
            reopened.invite(MemberRecord::for_test("alice")),
            Err(MembersError::NotFound(_))
        ));
    }

    #[test]
    fn a_damaged_record_stops_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("members.json");
        fs::write(&path, "{").unwrap();
        assert!(matches!(
            Members::open(&path),
            Err(MembersError::Corrupt { .. })
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{", "不覆盖坏记录");
    }

    #[test]
    fn removed_members_get_no_codes_and_joining_emits_events() {
        let dir = tempfile::tempdir().unwrap();
        let members = Members::open(dir.path().join("members.json")).unwrap();
        let mut events = members.subscribe();
        members.invite(MemberRecord::for_test("bob")).unwrap();
        members.mark_joined("bob").unwrap();
        match events.try_recv() {
            Ok(MemberEvent::Joined { username }) => assert_eq!(username, "bob"),
            other => panic!("要发 Joined 事件：{other:?}"),
        }
        let code = members.issue_pairing_code("bob").unwrap();
        members.remove("bob", false, false).unwrap();
        match events.try_recv() {
            Ok(MemberEvent::Removed { username, .. }) => assert_eq!(username, "bob"),
            other => panic!("要发 Removed 事件：{other:?}"),
        }
        assert!(matches!(
            members.issue_pairing_code("bob"),
            Err(MembersError::Removed(_))
        ));
        let devices = crate::device::Devices::open(dir.path().join("devices.json")).unwrap();
        assert_eq!(
            members
                .request_join(
                    code.secret.expose(),
                    iroh::SecretKey::generate().public(),
                    "手机",
                    &devices,
                )
                .unwrap(),
            JoinCheck::Expired
        );
        assert!(
            members
                .update_profile(
                    "bob",
                    &OwnerProfileUpdate {
                        display_name: "新名字".to_owned(),
                        email: None,
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn profile_updates_follow_the_owner_rules() {
        let dir = tempfile::tempdir().unwrap();
        let members = Members::open(dir.path().join("members.json")).unwrap();
        let mut invited = MemberRecord::for_test("carol");
        invited.display_name = "卡罗尔".to_owned();
        members.invite(invited).unwrap();
        let filled = members
            .update_profile(
                "carol",
                &OwnerProfileUpdate {
                    display_name: " 卡罗尔 ".to_owned(),
                    email: Some(" carol@example.com ".to_owned()),
                },
            )
            .unwrap();
        assert_eq!(filled.display_name, "卡罗尔");
        assert_eq!(filled.email.as_deref(), Some("carol@example.com"));

        let bad = members.update_profile(
            "carol",
            &OwnerProfileUpdate {
                display_name: String::new(),
                email: None,
            },
        );
        assert!(bad.is_err());
        let clear = members.update_profile(
            "carol",
            &OwnerProfileUpdate {
                display_name: "卡罗尔".to_owned(),
                email: None,
            },
        );
        assert!(clear.is_err(), "邮箱填过以后不能清空");
        let change = members.update_profile(
            "carol",
            &OwnerProfileUpdate {
                display_name: "卡罗尔".to_owned(),
                email: Some("new@example.com".to_owned()),
            },
        );
        assert_eq!(change.unwrap().email.as_deref(), Some("new@example.com"));
    }

    #[test]
    fn username_candidates_come_from_ascii_display_names_only() {
        for (display_name, want) in [
            ("Alice", Some("alice")),
            ("Alice-2_Girl", Some("alice-2_girl")),
            ("Alice!", Some("alice")),
            ("Alice 的 iPhone", None),
            ("张三", None),
            ("123", None),
            ("", None),
        ] {
            assert_eq!(
                username_candidate(display_name).as_deref(),
                want,
                "{display_name}"
            );
        }
        let invite: MemberInvite = serde_json::from_value(serde_json::json!({
            "display_name": "爱丽丝",
            "private_bot": true,
        }))
        .unwrap();
        assert_eq!(
            invite.username, None::<String>,
            "协议里没填用户名时核心生成"
        );
    }

    #[test]
    fn family_usernames_take_the_smallest_free_number() {
        let taken = |name: &str| name == "family1" || name == "family3";
        assert_eq!(family_username(taken), "family2");
        let taken = |name: &str| name == "family1" || name == "family2" || name == "family3";
        assert_eq!(family_username(taken), "family4");
        assert_eq!(family_username(|_| false), "family1");
    }
}
