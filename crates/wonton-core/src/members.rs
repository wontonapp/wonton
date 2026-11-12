//! 家人（ADR-0045）：记录在 `core/members.json`，写法照 `device.rs`：整份重写，0600，启动时读回。
//! 已移除的家人留在表里，用户名不再分配。归家人的配对码只存密钥的 SHA-256 和过期时间，随记录落盘，
//! 核心重启后还在；码有效一次，配上就清掉。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wonton_protocol::{
    MEMBER_PAIRING_CODE_TTL_SECS, MemberStatus, OwnerProfileUpdate, check_display_name,
    check_email_change, normalized_email,
};

use wonton_tunnel::pair::{PairingSecret, SECRET_LEN};

use crate::owner;

fn invalid(message: String) -> MembersError {
    MembersError::Invalid(message)
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
}

/// 后续任务订阅的家人事件：第一台设备配上（建私人 Bot、Wonton 私聊、拉进家庭群）和移除家人
/// （停用 Matrix 账号、删 Bot、删私人空间）。发送失败（还没有订阅者）就丢弃，事件只是通知。
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
    /// 主人授权了“能装插件”。
    #[serde(default)]
    pub can_install_plugins: bool,
    /// 邀请时主人选了建私人 Bot；建 Bot 是后续任务，先记下来。
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

#[derive(Serialize, Deserialize)]
struct Stored {
    members: Vec<MemberRecord>,
}

/// 用归家人的码来配对时，密钥查出来的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeCheck {
    /// 码归这个家人。
    Match(String),
    Expired,
    Unknown,
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
    members: Mutex<Vec<MemberRecord>>,
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
            Ok(text) => {
                let stored: Stored =
                    serde_json::from_str(&text).map_err(|e| MembersError::Corrupt {
                        path: path.clone(),
                        message: e.to_string(),
                    })?;
                stored.members
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
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
        self.lock().sort_by(|a, b| a.username.cmp(&b.username));
        self.lock().clone()
    }

    pub fn get(&self, username: &str) -> Option<MemberRecord> {
        self.lock().iter().find(|m| m.username == username).cloned()
    }

    /// 这个用户名是不是任何一条家人记录（包括已移除的）。
    pub fn taken(&self, username: &str) -> bool {
        self.lock().iter().any(|m| m.username == username)
    }

    /// 记下一个“已邀请”的家人。用户名的查重由调用方做完，这里只拦记录里已经有的。
    pub fn invite(&self, record: MemberRecord) -> Result<MemberRecord, MembersError> {
        let mut members = self.lock();
        if members.iter().any(|m| m.username == record.username) {
            return Err(MembersError::NotFound(record.username));
        }
        let record = MemberRecord {
            status: MemberStatus::Invited,
            ..record
        };
        self.save(&mut members, record.clone())?;
        Ok(record)
    }

    /// 主人改一个家人的设置。
    pub fn set_settings(
        &self,
        username: &str,
        can_install_plugins: bool,
    ) -> Result<MemberRecord, MembersError> {
        self.update(username, |record| {
            record.can_install_plugins = can_install_plugins;
            Ok(())
        })
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
        let mut members = self.lock();
        let record = members
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
        let stored = Stored {
            members: members.clone(),
        };
        write_record(&self.path, &stored)?;
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

    /// 配对入口查码：密钥对得上、没过期、家人没被移除才算数。改记录在配上之后（[`Self::consume_code`]）。
    pub fn verify_code(&self, secret_hex: &str) -> CodeCheck {
        let hash = code_hash(secret_hex);
        let now_ms = (self.now_ms)();
        let members = self.lock();
        let Some(record) = members
            .iter()
            .find(|m| m.pairing_code_hash.as_deref() == Some(hash.as_str()))
        else {
            return CodeCheck::Unknown;
        };
        if record.status == MemberStatus::Removed
            || record
                .pairing_code_expires_at_ms
                .is_none_or(|at| at <= now_ms)
        {
            return CodeCheck::Expired;
        }
        CodeCheck::Match(record.username.clone())
    }

    /// 家人的码配对：查码、核对、清码在同一把锁里完成。码是一次性的：两台设备同时拿同一个码，
    /// 只有一台拿到 `Match`，另一台看到码已经不在了。返回用户名后在外面登记设备；登记失败时
    /// 码已经作废，主人重发一个。写盘失败时不放行（`Err`）：内存里的码照样清掉，但盘上还在的
    /// 旧码核心重启后会复活，不能再配出一台设备。
    pub fn take_code(&self, secret_hex: &str) -> Result<CodeCheck, MembersError> {
        let hash = code_hash(secret_hex);
        let now_ms = (self.now_ms)();
        let mut members = self.lock();
        let Some(record) = members
            .iter_mut()
            .find(|m| m.pairing_code_hash.as_deref() == Some(hash.as_str()))
        else {
            return Ok(CodeCheck::Unknown);
        };
        if record.status == MemberStatus::Removed
            || record
                .pairing_code_expires_at_ms
                .is_none_or(|at| at <= now_ms)
        {
            return Ok(CodeCheck::Expired);
        }
        let username = record.username.clone();
        record.pairing_code_hash = None;
        record.pairing_code_expires_at_ms = None;
        let stored = Stored {
            members: members.clone(),
        };
        write_record(&self.path, &stored)?;
        Ok(CodeCheck::Match(username))
    }

    /// 配上之后清码；第一次用上时家人从“已邀请”改成“已加入”，发 [`MemberEvent::Joined`]。
    pub fn consume_code(&self, username: &str) -> Result<bool, MembersError> {
        let mut joined = false;
        self.update(username, |record| {
            record.pairing_code_hash = None;
            record.pairing_code_expires_at_ms = None;
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
        let mut record = None;
        self.update(username, |member| {
            member.status = MemberStatus::Removed;
            // 配对码的哈希留着：verify_code 对“已移除”的记录回 Expired，码作废但不是不存在的码。
            record = Some(member.clone());
            Ok(())
        })?;
        let record = record.unwrap_or_else(|| unreachable!("update 找到了这条记录"));
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
        let Some(record) = members.iter_mut().find(|m| m.username == username) else {
            return Err(MembersError::NotFound(username.to_owned()));
        };
        change(record)?;
        // 先复制要返回的记录，可变借用就此结束，整份快照才能克隆出来。
        let updated = members
            .iter()
            .find(|m| m.username == username)
            .cloned()
            .expect("update 找到了这条记录");
        let stored = Stored {
            members: members.clone(),
        };
        write_record(&self.path, &stored)?;
        Ok(updated)
    }

    fn save(
        &self,
        members: &mut Vec<MemberRecord>,
        record: MemberRecord,
    ) -> Result<(), MembersError> {
        members.push(record);
        let stored = Stored {
            members: members.clone(),
        };
        if let Err(e) = write_record(&self.path, &stored) {
            members.pop();
            return Err(e);
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<MemberRecord>> {
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
mod tests {
    use super::*;
    use wonton_protocol::MemberInvite;

    fn record(username: &str) -> MemberRecord {
        MemberRecord {
            username: username.to_owned(),
            user_id: format!("@{username}:wonton.localhost"),
            display_name: username.to_owned(),
            email: None,
            status: MemberStatus::Invited,
            can_install_plugins: false,
            wants_private_bot: true,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        }
    }

    #[test]
    fn invite_survives_a_restart_and_removed_usernames_are_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("members.json");
        let members = Members::open(&path).unwrap();
        members.invite(record("alice")).unwrap();
        members.set_settings("alice", true).unwrap();
        members.remove("alice", false, true).unwrap();

        let reopened = Members::open(&path).unwrap();
        let alice = reopened.get("alice").unwrap();
        assert_eq!(alice.status, MemberStatus::Removed);
        assert!(alice.can_install_plugins, "设置跟着记录走");
        assert!(reopened.taken("alice"), "移除后用户名不再分配");
        assert!(matches!(
            reopened.invite(record("alice")),
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
    fn two_devices_racing_on_one_code_leave_one_match() {
        let dir = tempfile::tempdir().unwrap();
        let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
        members.invite(record("alice")).unwrap();
        let code = members.issue_pairing_code("alice").unwrap();
        let secret = code.secret.expose().to_owned();

        // 两台设备同时拿同一个码：只有一台配上，另一台看到码已经不在了。
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let members = members.clone();
                let secret = secret.clone();
                std::thread::spawn(move || members.take_code(&secret))
            })
            .collect();
        let results: Vec<CodeCheck> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();
        let matches = results
            .iter()
            .filter(|check| matches!(check, CodeCheck::Match(_)))
            .count();
        assert_eq!(matches, 1, "{results:?}");
        assert!(
            results
                .iter()
                .any(|check| matches!(check, CodeCheck::Unknown)),
            "{results:?}"
        );
        // 码已经清掉：再拿一次是不存在的码。
        assert!(matches!(
            members.take_code(&secret).unwrap(),
            CodeCheck::Unknown
        ));
    }

    #[test]
    fn a_failed_write_does_not_pair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("members.json");
        let members = Members::open(&path).unwrap();
        members.invite(record("alice")).unwrap();
        let code = members.issue_pairing_code("alice").unwrap();
        // 记录目录只读：写盘失败。
        std::fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o555),
        )
        .unwrap();
        assert!(members.take_code(code.secret.expose()).is_err());
        // 内存里的码照样清掉：恢复写权限后，再拿同一个码是不存在的码。
        std::fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        assert!(matches!(
            members.take_code(code.secret.expose()).unwrap(),
            CodeCheck::Unknown
        ));
    }

    #[test]
    fn pairing_codes_are_hashed_single_use_and_expire_by_the_injected_clock() {
        let dir = tempfile::tempdir().unwrap();
        let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000_000));
        let members = Members::with_clock(
            dir.path().join("members.json"),
            Arc::new({
                let now = now.clone();
                move || now.load(std::sync::atomic::Ordering::SeqCst)
            }),
        )
        .unwrap();
        members.invite(record("alice")).unwrap();

        let code = members.issue_pairing_code("alice").unwrap();
        assert_eq!(
            members.list()[0].pairing_code_hash.as_deref(),
            Some(code_hash(code.secret.expose()).as_str()),
            "存的是哈希，不是密钥"
        );
        let text = fs::read_to_string(dir.path().join("members.json")).unwrap();
        assert!(!text.contains(code.secret.expose()), "码的原文不落盘");

        assert_eq!(
            members.verify_code(code.secret.expose()),
            CodeCheck::Match("alice".to_owned())
        );
        members.consume_code("alice").unwrap();
        assert_eq!(members.get("alice").unwrap().status, MemberStatus::Joined);
        assert_eq!(
            members.verify_code(code.secret.expose()),
            CodeCheck::Unknown,
            "码只能用一次"
        );

        // 再出一个，旧的作废；24 小时后不能用。
        let second = members.issue_pairing_code("alice").unwrap();
        assert_eq!(
            members.verify_code(code.secret.expose()),
            CodeCheck::Unknown,
            "再出一个以后旧的不能用"
        );
        now.store(
            1_000_000 + MEMBER_PAIRING_CODE_TTL_SECS * 1000 + 1,
            std::sync::atomic::Ordering::SeqCst,
        );
        assert_eq!(
            members.verify_code(second.secret.expose()),
            CodeCheck::Expired,
            "过了 24 小时不能用"
        );
        members.void_pairing_code("alice").unwrap();
        assert_eq!(members.get("alice").unwrap().pairing_code_hash, None);
    }

    #[test]
    fn removed_members_get_no_codes_and_joining_emits_events() {
        let dir = tempfile::tempdir().unwrap();
        let members = Members::open(dir.path().join("members.json")).unwrap();
        let mut events = members.subscribe();
        members.invite(record("bob")).unwrap();
        members.consume_code("bob").unwrap();
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
        assert_eq!(
            members.verify_code(code.secret.expose()),
            CodeCheck::Expired,
            "移除时码作废"
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
        let mut invited = record("carol");
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
            ("Jiale 的 iPhone", None),
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
