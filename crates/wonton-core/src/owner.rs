//! 主人的身份：用户名、显示名、邮箱和时区的规则，以及记进数据目录的记录。用户名装好后不变（ADR-0022），显示名和邮箱在
//! App 里填（ADR-0030）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use wonton_protocol::{
    OwnerProfile, OwnerProfileUpdate, ProfileProblem, check_display_name, check_email,
    check_email_change, normalized_email,
};

/// 没配 `WONTON_OWNER_NAME`、数据目录里也没有记录时的用户名。
pub const DEFAULT_USERNAME: &str = "owner";

const RECORD: &str = "owner.json";

/// Debian、Alpine 基础镜像 `/etc/passwd` 里的名字和 `nogroup`。插件的镜像脚本拿用户名建账号时，和它们重名就成了去改
/// 那个系统用户。
const SYSTEM_NAMES: &[&str] = &[
    "adm",
    "at",
    "backup",
    "bin",
    "cron",
    "cyrus",
    "daemon",
    "ftp",
    "games",
    "gnats",
    "guest",
    "halt",
    "irc",
    "list",
    "lp",
    "mail",
    "man",
    "news",
    "nobody",
    "nogroup",
    "ntp",
    "operator",
    "postmaster",
    "proxy",
    "root",
    "shutdown",
    "smmsp",
    "squid",
    "sshd",
    "sync",
    "sys",
    "uucp",
    "vpopmail",
    "www-data",
    "xfs",
];

/// 主人或家人的身份；真实邮箱只用于资料读写，插件身份统一派生内部邮箱。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub username: String,
    /// 还没填时是用户名：应用要显示点什么，用户名是真的，不另外编一个。
    pub display_name: String,
    /// 资料页保存的真实邮箱，不向插件公开（ADR-0056）。
    pub email: Option<String>,
}

/// 用户名固定且不复用，内部地址因此不受真实邮箱修改影响（ADR-0056）。
pub fn internal_email(username: &str) -> String {
    format!("{username}@wonton.internal")
}

/// 好友的内部邮箱：只认人不收信，按固定的本机用户名派生，删好友再加回也不变、不给人重复用。
pub fn friend_email(username: &str) -> String {
    internal_email(username)
}

/// 可移植的 POSIX 用户名（`^[a-z][a-z0-9_-]{0,31}$`），同时是合法的 Matrix localpart。
pub fn check_username(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    let starts_with_letter = bytes.next().is_some_and(|b| b.is_ascii_lowercase());
    let valid = starts_with_letter
        && name.len() <= 32
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !valid {
        return Err("只能有小写字母、数字、_ 和 -，以字母开头，最长 32 个字符".to_owned());
    }
    Ok(())
}

/// 保留名单只在第一次记下用户名时查：名单以后会加名字，加了不能让已有的主机起不来，用户名又改不了。
pub fn check_not_reserved(name: &str) -> Result<(), String> {
    if SYSTEM_NAMES.contains(&name) {
        return Err(format!(
            "{name} 是系统用户名，插件拿它建账号时会改到那个系统用户：换一个名字"
        ));
    }
    if name == crate::matrix::ADMIN_LOCALPART || name == crate::matrix::SERVER_USER_LOCALPART {
        return Err(format!(
            "{name} 是 Matrix 服务器自用的账号，登录会进到那个账号：换一个名字"
        ));
    }
    Ok(())
}

/// 只接受 IANA 时区名。POSIX 写法（`CST-8`）和 `:/etc/localtime` 让用严格时区库的插件起不来。时区库内嵌在核心里：
/// 镜像里不一定有 `/usr/share/zoneinfo`。
pub fn check_time_zone(tz: &str) -> Result<(), String> {
    // `get` 不分大小写，名字要和时区库里的完全一致。
    match jiff_tzdb::get(tz) {
        Some((name, _)) if name == tz => Ok(()),
        _ => Err("要是 IANA 时区名，如 Asia/Shanghai、UTC".to_owned()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error(
        "WONTON_OWNER_NAME 是 {configured}，但这台服务器的用户名是 {recorded}（记在 {}）。用户名装好后不能改：\
         Matrix 账号和插件里的账号都挂在它上面。把 WONTON_OWNER_NAME 改回 {recorded} 或者删掉它；要换用户名只能用新的\
         数据目录重新安装",
        path.display()
    )]
    Mismatch {
        configured: String,
        recorded: String,
        path: PathBuf,
    },
    #[error("WONTON_OWNER_NAME 不能用：{0}")]
    Reserved(String),
    #[error("身份记录 {} 读不了：{message}。核心不覆盖它：从备份恢复这个文件", path.display())]
    Unreadable { path: PathBuf, message: String },
    #[error("写不进身份记录 {}：{source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    username: String,
    display_name: Option<String>,
    email: Option<String>,
}

impl Record {
    fn check(&self) -> Result<(), String> {
        check_username(&self.username)?;
        if let Some(name) = &self.display_name {
            check_display_name(name).map_err(|problem| problem.to_string())?;
        }
        if let Some(email) = &self.email {
            check_email(email).map_err(|problem| problem.to_string())?;
        }
        Ok(())
    }
}

fn invalid(problem: ProfileProblem) -> ProfileError {
    ProfileError::Invalid(problem.to_string())
}

/// 保存资料失败。
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("{0}")]
    Invalid(String),
    #[error("写不进身份记录 {}：{source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// 记在数据目录里的身份。显示名和邮箱随时会改，用的地方每次都从这里取，不留副本。
#[derive(Debug)]
pub struct OwnerStore {
    state_dir: PathBuf,
    username: String,
    profile: RwLock<(Option<String>, Option<String>)>,
}

impl OwnerStore {
    /// 读 [`lock_username`] 记下的身份。
    pub fn open(state_dir: &Path) -> Result<Arc<Self>, LockError> {
        let record = read_record(state_dir)?.ok_or_else(|| LockError::Unreadable {
            path: state_dir.join(RECORD),
            message: "文件不在".to_owned(),
        })?;
        Ok(Arc::new(Self {
            state_dir: state_dir.to_owned(),
            username: record.username,
            profile: RwLock::new((record.display_name, record.email)),
        }))
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn owner(&self) -> Owner {
        let profile = self.profile.read().expect("身份记录锁中毒");
        Owner {
            username: self.username.clone(),
            display_name: profile.0.clone().unwrap_or_else(|| self.username.clone()),
            email: profile.1.clone(),
        }
    }

    pub fn email(&self) -> Option<String> {
        self.profile.read().expect("身份记录锁中毒").1.clone()
    }

    pub fn profile(&self) -> OwnerProfile {
        let profile = self.profile.read().expect("身份记录锁中毒");
        OwnerProfile {
            username: self.username.clone(),
            display_name: profile.0.clone(),
            email: profile.1.clone(),
        }
    }

    /// 校验、写盘、再换内存里的值：写不进去时还是原来的身份。
    pub fn update(&self, update: &OwnerProfileUpdate) -> Result<OwnerProfile, ProfileError> {
        let display_name = update.display_name.trim();
        let email = normalized_email(update.email.as_deref());
        check_display_name(display_name).map_err(invalid)?;
        let mut profile = self.profile.write().expect("身份记录锁中毒");
        check_email_change(profile.1.as_deref(), email).map_err(invalid)?;
        let record = Record {
            username: self.username.clone(),
            display_name: Some(display_name.to_owned()),
            email: email.map(str::to_owned),
        };
        write_record(&self.state_dir, &record).map_err(|source| ProfileError::Write {
            path: self.state_dir.join(RECORD),
            source,
        })?;
        *profile = (record.display_name, record.email);
        drop(profile);
        Ok(self.profile())
    }

    /// 测试用：`dir` 下一份写好的身份。
    #[cfg(test)]
    pub(crate) fn for_test(dir: &Path, owner: &Owner) -> Arc<Self> {
        let state_dir = dir.join("owner-record");
        let record = Record {
            username: owner.username.clone(),
            display_name: Some(owner.display_name.clone()),
            email: owner.email.clone(),
        };
        write_record(&state_dir, &record).unwrap();
        Self::open(&state_dir).unwrap()
    }
}

/// 定下主人的用户名：有记录时以记录为准，`configured` 和它不一致就报错；没有记录时取 `configured`（没配时是
/// [`DEFAULT_USERNAME`]）并记下来。`configured` 已经过 [`check_username`]。
pub fn lock_username(state_dir: &Path, configured: Option<&str>) -> Result<String, LockError> {
    if let Some(recorded) = recorded_username(state_dir, configured)? {
        return Ok(recorded);
    }
    let username = configured.unwrap_or(DEFAULT_USERNAME);
    check_not_reserved(username).map_err(LockError::Reserved)?;
    let record = Record {
        username: username.to_owned(),
        display_name: None,
        email: None,
    };
    write_record(state_dir, &record).map_err(|source| LockError::Write {
        path: state_dir.join(RECORD),
        source,
    })?;
    Ok(username.to_owned())
}

/// [`lock_username`] 的只读版本，什么也不写：安装脚本启动容器前用它当场报错。
pub fn check_before_install(state_dir: &Path, configured: Option<&str>) -> Result<(), LockError> {
    if recorded_username(state_dir, configured)?.is_none()
        && let Some(name) = configured
    {
        check_not_reserved(name).map_err(LockError::Reserved)?;
    }
    Ok(())
}

/// 有记录时返回它，`configured` 和它不一致就报错。
fn recorded_username(
    state_dir: &Path,
    configured: Option<&str>,
) -> Result<Option<String>, LockError> {
    let Some(record) = read_record(state_dir)? else {
        return Ok(None);
    };
    match configured {
        Some(configured) if configured != record.username => Err(LockError::Mismatch {
            configured: configured.to_owned(),
            recorded: record.username,
            path: state_dir.join(RECORD),
        }),
        _ => Ok(Some(record.username)),
    }
}

/// 记下的主人用户名；没有记录或读不了时为空。只读，不检查 `WONTON_OWNER_NAME`，给后端没有身份的路径用
/// （比如按数据目录找主人的语言）。
pub fn recorded_name(state_dir: &Path) -> Option<String> {
    match read_record(state_dir) {
        Ok(record) => record.map(|r| r.username),
        Err(error) => {
            tracing::warn!(%error, "读不出主人记录，按没有用户名的路径处理");
            None
        }
    }
}

fn read_record(state_dir: &Path) -> Result<Option<Record>, LockError> {
    let path = state_dir.join(RECORD);
    let unreadable = |message: String| LockError::Unreadable {
        path: path.clone(),
        message,
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(unreadable(e.to_string())),
    };
    let record: Record = serde_json::from_str(&text).map_err(|e| unreadable(e.to_string()))?;
    record.check().map_err(unreadable)?;
    Ok(Some(record))
}

/// 先写临时文件再改名：写到一半掉电时，下次启动不会读到半个记录。
fn write_record(state_dir: &Path, record: &Record) -> std::io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let text = serde_json::to_string(record).map_err(std::io::Error::other)?;
    let tmp = state_dir.join(format!("{RECORD}.tmp"));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, state_dir.join(RECORD))?;
    fs::File::open(state_dir)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_start_records_the_name_and_later_starts_must_match_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("core");
        assert_eq!(lock_username(&state, Some("alice")).unwrap(), "alice");
        assert_eq!(lock_username(&state, Some("alice")).unwrap(), "alice");
        // 没配不算冲突：`.env` 丢了也起得来，还是原来的人。
        assert_eq!(lock_username(&state, None).unwrap(), "alice");
        let error = lock_username(&state, Some("bob")).unwrap_err().to_string();
        for needed in ["bob", "alice", "owner.json", "重新安装"] {
            assert!(error.contains(needed), "{error}");
        }
        assert_eq!(lock_username(&state, None).unwrap(), "alice", "记录没被改");
    }

    #[test]
    fn without_a_setting_the_name_is_owner_and_it_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(lock_username(dir.path(), None).unwrap(), "owner");
        assert!(lock_username(dir.path(), Some("alice")).is_err());
    }

    #[test]
    fn a_damaged_record_stops_the_start_and_is_not_overwritten() {
        for text in ["", "{", r#"{"username":"Bad/Name"}"#] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(RECORD);
            fs::write(&path, text).unwrap();
            let error = lock_username(dir.path(), Some("alice")).unwrap_err();
            assert!(matches!(error, LockError::Unreadable { .. }), "{text}");
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }

    fn filled_in(
        store: &OwnerStore,
        name: &str,
        email: &str,
    ) -> Result<OwnerProfile, ProfileError> {
        store.update(&OwnerProfileUpdate {
            display_name: name.to_owned(),
            email: Some(email.to_owned()),
        })
    }

    #[test]
    fn a_new_identity_has_only_the_username_and_filling_it_in_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        lock_username(dir.path(), Some("alice")).unwrap();
        let store = OwnerStore::open(dir.path()).unwrap();
        assert_eq!(store.profile().display_name, None);
        assert_eq!(store.owner().display_name, "alice");
        assert_eq!(store.owner().email, None);

        let saved = filled_in(&store, "  爱丽丝 ", " alice@example.com ").unwrap();
        assert_eq!(saved.display_name.as_deref(), Some("爱丽丝"));
        assert_eq!(saved.email.as_deref(), Some("alice@example.com"));

        lock_username(dir.path(), Some("alice")).unwrap();
        let reopened = OwnerStore::open(dir.path()).unwrap();
        assert_eq!(reopened.owner().display_name, "爱丽丝");
        assert_eq!(reopened.email().as_deref(), Some("alice@example.com"));
        assert_eq!(reopened.username(), "alice");
    }

    #[test]
    fn a_bad_update_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        lock_username(dir.path(), None).unwrap();
        let store = OwnerStore::open(dir.path()).unwrap();
        filled_in(&store, "你", "me@example.com").unwrap();
        let too_long = "字".repeat(65);
        for (name, email) in [
            ("", "new@example.com"),
            ("  ", "new@example.com"),
            (too_long.as_str(), "new@example.com"),
            ("a\nb", "new@example.com"),
            ("新", "not-an-email"),
        ] {
            assert!(matches!(
                filled_in(&store, name, email),
                Err(ProfileError::Invalid(_))
            ));
        }
        assert_eq!(store.owner().display_name, "你");
        assert_eq!(store.email().as_deref(), Some("me@example.com"));
    }

    #[test]
    fn the_email_is_optional_until_it_is_filled_in() {
        let dir = tempfile::tempdir().unwrap();
        lock_username(dir.path(), None).unwrap();
        let store = OwnerStore::open(dir.path()).unwrap();
        let saved = filled_in(&store, "你", "  ").unwrap();
        assert_eq!(saved.display_name.as_deref(), Some("你"));
        assert_eq!(saved.email, None);

        filled_in(&store, "你", "me@example.com").unwrap();
        assert!(matches!(
            filled_in(&store, "你", ""),
            Err(ProfileError::Invalid(_))
        ));
        assert_eq!(store.email().as_deref(), Some("me@example.com"));
    }

    #[test]
    fn a_record_with_a_bad_email_stops_the_start() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(RECORD),
            r#"{"username":"owner","display_name":null,"email":"not-an-email"}"#,
        )
        .unwrap();
        assert!(matches!(
            lock_username(dir.path(), None),
            Err(LockError::Unreadable { .. })
        ));
    }

    #[test]
    fn checking_before_install_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        check_before_install(dir.path(), Some("alice")).unwrap();
        assert!(!dir.path().join(RECORD).exists());
    }

    #[test]
    fn reserved_names_are_refused_only_when_first_recorded() {
        for reserved in ["root", "guest", "nogroup", "wonton-core", "conduit"] {
            let dir = tempfile::tempdir().unwrap();
            for error in [
                lock_username(dir.path(), Some(reserved)).unwrap_err(),
                check_before_install(dir.path(), Some(reserved)).unwrap_err(),
            ] {
                assert!(matches!(error, LockError::Reserved(_)), "{reserved}");
            }
            assert!(!dir.path().join(RECORD).exists());

            // 名字是后来才进名单的：已有的主机照常起来。
            fs::write(
                dir.path().join(RECORD),
                format!(r#"{{"username":"{reserved}"}}"#),
            )
            .unwrap();
            assert_eq!(lock_username(dir.path(), Some(reserved)).unwrap(), reserved);
            assert_eq!(lock_username(dir.path(), None).unwrap(), reserved);
            check_before_install(dir.path(), Some(reserved)).unwrap();
        }
    }

    #[test]
    fn usernames_are_portable() {
        for good in ["owner", "alice", "a", "al_ice-1", &"a".repeat(32)] {
            assert_eq!(check_username(good), Ok(()), "{good}");
        }
        let too_long = "a".repeat(33);
        for bad in [
            "", "Alice", "a/b", "a+b", "a=b", "a.b", "a b", "1a", "_a", "-a", "张三", &too_long,
        ] {
            assert!(check_username(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_iana_time_zone_names_are_accepted() {
        for good in ["UTC", "Asia/Shanghai", "America/New_York", "Etc/UTC"] {
            assert_eq!(check_time_zone(good), Ok(()), "{good}");
        }
        for bad in ["", "CST-8", ":/etc/localtime", "asia/shanghai", "Mars/Base"] {
            assert!(check_time_zone(bad).is_err(), "{bad}");
        }
    }
}
