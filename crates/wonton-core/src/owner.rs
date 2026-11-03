//! 主人的身份（ADR-0022）：用户名、邮箱和时区的规则，以及把用户名记进数据目录。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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

/// 主人还没配邮箱、插件又要用时的错误，写明怎么修。`docker restart` 不重新读 `.env`。
pub const NO_EMAIL: &str = "主人还没配邮箱：在安装目录的 .env 里写上 WONTON_OWNER_EMAIL，再执行一次 install.sh 或 docker compose up -d";

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

/// 不验证能不能收信，只挡住明显不是邮箱的值：它会原样进 OIDC 声明和请求头。
pub fn check_email(email: &str) -> Result<(), String> {
    let valid = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && !domain.is_empty() && !domain.contains('@')
    }) && !email.chars().any(|c| c.is_whitespace() || c.is_control());
    if valid {
        Ok(())
    } else {
        Err("不像邮箱：写成 me@example.com 这样，没有空白和控制字符".to_owned())
    }
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
        "WONTON_OWNER_NAME 是 {configured}，但这台服务器的主人用户名是 {recorded}（记在 {}）。用户名装好后不能改：\
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
    #[error("主人用户名的记录 {} 读不了：{message}。核心不覆盖它：从备份恢复这个文件", path.display())]
    Unreadable { path: PathBuf, message: String },
    #[error("写不进主人用户名的记录 {}：{source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Serialize, Deserialize)]
struct Record {
    username: String,
}

/// 定下主人的用户名：有记录时以记录为准，`configured` 和它不一致就报错；没有记录时取 `configured`（没配时是
/// [`DEFAULT_USERNAME`]）并记下来。`configured` 已经过 [`check_username`]。
pub fn lock_username(state_dir: &Path, configured: Option<&str>) -> Result<String, LockError> {
    if let Some(recorded) = recorded_username(state_dir, configured)? {
        return Ok(recorded);
    }
    let username = configured.unwrap_or(DEFAULT_USERNAME);
    check_not_reserved(username).map_err(LockError::Reserved)?;
    write_record(state_dir, username).map_err(|source| LockError::Write {
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
    check_username(&record.username).map_err(unreadable)?;
    match configured {
        Some(configured) if configured != record.username => Err(LockError::Mismatch {
            configured: configured.to_owned(),
            recorded: record.username,
            path,
        }),
        _ => Ok(Some(record.username)),
    }
}

/// 先写临时文件再改名：写到一半掉电时，下次启动不会读到半个记录。
fn write_record(state_dir: &Path, username: &str) -> std::io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let record = Record {
        username: username.to_owned(),
    };
    let text = serde_json::to_string(&record).map_err(std::io::Error::other)?;
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
    fn emails_are_checked_for_shape_only() {
        assert_eq!(check_email("me@example.com"), Ok(()));
        assert_eq!(check_email("me@localhost"), Ok(()));
        for bad in ["", "me", "@example.com", "me@", "a@b@c", "a b@c", "a@b\n"] {
            assert!(check_email(bad).is_err(), "{bad:?}");
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
