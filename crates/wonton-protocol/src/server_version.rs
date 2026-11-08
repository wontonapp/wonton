//! 接口从哪个服务器版本起才有：App 连上的是旧服务器时，某个接口的 404 归成
//! [`CoreError::ServerTooOld`](crate::CoreError::ServerTooOld)，不只给引导用。
//!
//! 服务器版本从它的状态接口来（核心升级状态的 `current.version`）；取不到时按未知处理，不当成旧。
//! 只有表里的接口会被这样判定：别的 404 分不清是“服务器没有这个接口”还是“没有这个对象”。

use crate::{ME_PATH, ONBOARDING_PATH};

/// `path` 这个接口最少要的服务器版本；不在表里时为空。
///
/// 版本写引入这个接口的渠道版本（镜像的 `org.opencontainers.image.version` 标签）。接口被新版本改名或挪走时
/// 更新这里，把 404 和“服务器太旧”对上。
pub fn minimum_server_version(path: &str) -> Option<&'static str> {
    match path {
        // ADR-0045：`me` 取代了 `owner-profile`。
        ME_PATH => Some("0.1.0-beta.5"),
        // ADR-0034 的引导接口。
        ONBOARDING_PATH => Some("0.1.0-beta.2"),
        _ => None,
    }
}

/// `server` 比 `minimum` 旧。两边按 semver 比：预发布比同号的发布旧，`-beta.4` 比 `-beta.5` 旧，`+构建`
/// 不参与比较。开头的 `v` 先去掉。任一边读不懂时返回假：不知道就不判旧。
pub fn is_older(server: &str, minimum: &str) -> bool {
    let (Some(server), Some(minimum)) = (parse(server), parse(minimum)) else {
        return false;
    };
    server < minimum
}

/// 少写段的版本（如 `0.1`）补零成三段再交给 semver；多写的照旧读不懂。
fn parse(version: &str) -> Option<semver::Version> {
    let version = version.trim().trim_start_matches('v');
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let mut parts: Vec<&str> = core.split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    let mut padded = parts.join(".");
    if let Some(pre) = pre {
        padded.push('-');
        padded.push_str(pre);
    }
    semver::Version::parse(&padded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_paths_have_a_minimum() {
        assert_eq!(minimum_server_version(ME_PATH), Some("0.1.0-beta.5"));
        assert_eq!(minimum_server_version("/_wonton/api/owner-profile"), None);
        assert_eq!(
            minimum_server_version(ONBOARDING_PATH),
            Some("0.1.0-beta.2")
        );
    }

    #[test]
    fn prereleases_and_numbers_order_like_versions() {
        assert!(is_older("0.1.0-beta.4", "0.1.0-beta.5"));
        assert!(is_older("0.1.0-beta.9", "0.1.0-beta.10"));
        assert!(is_older("0.1.0-beta.5", "0.1.0"));
        assert!(is_older("0.1.0", "0.1.1"));
        assert!(is_older("v0.1.0-beta.4", "0.1.0-beta.5"));
        // 构建记号不参与比较。
        assert!(!is_older("0.1.0-beta.5+build.1", "0.1.0-beta.5"));
        // 少写段的版本按补零算：`0.1` 等于 `0.1.0`。
        assert!(!is_older("0.1", "0.1.0"));
        assert!(is_older("0.1", "0.1.1"));
        assert!(is_older("0.1-beta.4", "0.1.0"));
        assert!(!is_older("0.1.0-beta.5", "0.1.0-beta.5"));
        assert!(!is_older("0.1.0", "0.1.0-beta.5"));
        assert!(!is_older("0.2.0", "0.1.0"));
    }

    #[test]
    fn unreadable_versions_are_not_older() {
        assert!(!is_older("", "0.1.0"));
        assert!(!is_older("main", "0.1.0"));
        assert!(!is_older("0.1.0", "garbage"));
    }
}
