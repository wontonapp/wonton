//! 工具在总线上的名字：插件的工具是 `<slug>__<工具名>`，核心自己的是 `wonton__<工具名>`。插件 ID 只有小写字母、
//! 数字、`.` 和单个 `-`，slug 里没有下划线，按第一个 `__` 拆开不会有歧义；插件 ID 至少两段，slug 里一定有 `-`，
//! 也不会和 `wonton` 重名。

/// 核心自己的工具的前缀。
pub const CORE: &str = "wonton";

const SEPARATOR: &str = "__";

pub fn join(owner: &str, tool: &str) -> String {
    format!("{owner}{SEPARATOR}{tool}")
}

/// 拆成（前缀，工具名）。
pub fn split(name: &str) -> Option<(&str, &str)> {
    name.split_once(SEPARATOR)
        .filter(|(owner, tool)| !owner.is_empty() && !tool.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_at_the_first_separator() {
        assert_eq!(
            split("app-files__search_files"),
            Some(("app-files", "search_files"))
        );
        assert_eq!(split("app-files__a__b"), Some(("app-files", "a__b")));
        assert_eq!(split("search_files"), None);
        assert_eq!(split("__x"), None);
        assert_eq!(join(CORE, "list_plugins"), "wonton__list_plugins");
    }
}
