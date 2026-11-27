use std::fmt;
use std::io;
use std::str::FromStr;

/// 在跑作业的归属：pid 加进程启动时间。
///
/// 只记 pid 时，重启后新进程复用同一 pid 会被当成原作业还活着；启动时间
/// 取自 `/proc/<pid>/stat` 的 starttime，对不上就按死处理。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Owner {
    pub pid: u32,
    pub start: u64,
}

impl Owner {
    pub fn current() -> io::Result<Self> {
        let pid = std::process::id();
        let start = match process_start(pid) {
            Ok(start) => start,
            // 没有 `/proc` 的平台存 0，判活永远按死处理。
            Err(err) if err.kind() == io::ErrorKind::Unsupported => 0,
            Err(err) => return Err(err),
        };
        Ok(Self { pid, start })
    }

    /// 存活：启动时间对得上才是同一个进程。读不到 `/proc` 条目说明进程没了；
    /// stat 因权限等原因读不了时按活着算，宁可等也不误杀。
    pub fn alive(&self) -> bool {
        match process_start(self.pid) {
            Ok(start) => start == self.start,
            Err(err) if err.kind() == io::ErrorKind::NotFound => false,
            Err(err) if err.kind() == io::ErrorKind::Unsupported => false,
            Err(_) => true,
        }
    }
}

/// 进程启动时间（自开机起的时钟滴答数）。Linux 以外没有 `/proc`，统一报不支持，
/// 于是 pid 判活一律按死接管。
pub fn process_start(pid: u32) -> io::Result<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let rest = stat
            .rfind(')')
            .map(|index| &stat[index + 1..])
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stat 没有 comm 结束括号"))?;
        // 第 3 个字段是 state；starttime 是第 22 个字段，即剩余字段里的第 20 个。
        rest.split_whitespace()
            .nth(19)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stat 缺 starttime"))?
            .parse()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Err(io::ErrorKind::Unsupported.into())
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pid:{}:{}", self.pid, self.start)
    }
}

impl FromStr for Owner {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts = text.split(':');
        let (Some("pid"), Some(pid), Some(start), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("owner 不是 pid:<pid>:<start>：{text}"));
        };
        Ok(Self {
            pid: pid.parse().map_err(|_| format!("pid 非法：{pid}"))?,
            start: start.parse().map_err(|_| format!("start 非法：{start}"))?,
        })
    }
}

impl From<Owner> for String {
    fn from(owner: Owner) -> Self {
        owner.to_string()
    }
}

impl TryFrom<String> for Owner {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive_and_round_trips() {
        let owner = Owner::current().unwrap();
        assert!(owner.alive());
        assert_eq!(owner.to_string().parse::<Owner>().unwrap(), owner);
    }

    #[test]
    fn reused_pid_with_other_start_is_dead() {
        let mut owner = Owner::current().unwrap();
        owner.start += 1;
        assert!(!owner.alive());
    }

    #[test]
    fn dead_pid_is_not_alive() {
        let mut owner = Owner::current().unwrap();
        owner.pid = 4_000_000;
        assert!(!owner.alive());
    }
}
