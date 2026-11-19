//! 这份核心的版本和提交号，构建镜像时由 `deploy/dev/core/Dockerfile` 的 build-arg 编进二进制。
//!
//! workspace 的 `version` 固定是 `0.0.0`，不能拿 `CARGO_PKG_VERSION` 当核心的版本。

use std::sync::LazyLock;

/// 发版的版本号，如 `0.3.0`（不带 `v`）：release.yml 构建发版镜像时传 `WONTON_VERSION`。
/// main 镜像、开发镜像和直接 `cargo build` 的都没有。
pub const RELEASE: Option<&str> = non_empty(option_env!("WONTON_VERSION"));

/// 构建它的提交号：CI 传完整的 sha，`dev.sh` 传短的（工作区有改动时带 `-dirty`）。直接 `cargo build` 时没有。
pub const COMMIT: Option<&str> = non_empty(option_env!("WONTON_BUILD_COMMIT"));

/// 对外报的版本（MCP 的 server info、User-Agent、`--version`）。
pub fn version() -> &'static str {
    static VERSION: LazyLock<String> = LazyLock::new(|| describe(RELEASE, COMMIT));
    &VERSION
}

/// 发版是版本号本身；开发版是 `dev+<提交号>`（semver 的构建元数据写法），不知道提交号时是 `dev`。
/// 不写成 `0.0.0` 之类像版本号的值，免得看的人以为是正式版。
fn describe(release: Option<&str>, commit: Option<&str>) -> String {
    match (release, commit) {
        (Some(release), _) => release.to_owned(),
        (None, Some(commit)) => format!("dev+{}", short_commit(commit)),
        (None, None) => "dev".to_owned(),
    }
}

/// 完整的 40 位 sha 取前 7 位；`dev.sh` 传来的短提交号（可能带 `-dirty`）原样用。
pub fn short_commit(commit: &str) -> &str {
    if commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        &commit[..7]
    } else {
        commit
    }
}

/// Dockerfile 里没传的 build-arg 是空串，和没有一样。
const fn non_empty(value: Option<&'static str>) -> Option<&'static str> {
    match value {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_build_reports_its_version() {
        assert_eq!(
            describe(
                Some("0.3.0"),
                Some("9d5604c95d0a1b2c3d4e5f60718293a4b5c6d7e8")
            ),
            "0.3.0"
        );
    }

    #[test]
    fn dev_build_reports_short_commit_not_a_version_number() {
        assert_eq!(
            describe(None, Some("9d5604c95d0a1b2c3d4e5f60718293a4b5c6d7e8")),
            "dev+9d5604c"
        );
        assert_eq!(describe(None, Some("9d5604c-dirty")), "dev+9d5604c-dirty");
        assert_eq!(describe(None, None), "dev");
    }
}
