//! 联邦名单文件 `secrets/tuwunel/wonton-fed.toml` 的生成（ADR-0050）。纯函数，运行时在 [super]。

use std::collections::BTreeSet;

/// 名单文件的内容：每个配对过的服务器名一个锚定正则。
pub(crate) fn allowlist_toml(names: &BTreeSet<String>) -> String {
    // RegexSet 的匹配是子串式的，不锚定的话 `a.wonton.internal` 会把 `xxa.wonton.internal` 也放进来。
    let patterns: Vec<String> = if names.is_empty() {
        // 服务器名的标签只能用小写字母、数字和连字符（install.rs 的 validate_server_name），
        // 带下划线的名字不可能存在，空名单就用它表示“谁也不匹配”。
        vec!["^_unpaired_\\.wonton\\.internal$".to_owned()]
    } else {
        names
            .iter()
            .map(|name| format!("^{}$", regex_escape(name)))
            .collect()
    };
    let list = patterns
        .iter()
        .map(|pattern| format!("'{pattern}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "# 核心运行时原地重写这份名单（不换 inode），改动经 SIGUSR1 热加载（ADR-0050）。\n\
         # 这份文件只写联邦名单，别的选项在 tuwunel.toml。\n\
         allowed_remote_server_names_experimental = [{list}]\n"
    )
}

/// 服务器名进正则：名字经过校验只有 `[a-z0-9.-]`，点要转义；其他符号也一并转义，转义字母和数字
/// 在正则里是错的，不会出现。
pub(crate) fn regex_escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use regex::RegexSet;

    use super::allowlist_toml;

    fn patterns(list: &str) -> Vec<String> {
        // 生成的行形如 `allowed_remote_server_names_experimental = ['…', '…']`，取出方括号里的字面量。
        let values = list
            .split(" = [")
            .nth(1)
            .expect("有名单")
            .trim_end_matches("]\n");
        values
            .split(", ")
            .map(|item| item.trim_matches('\'').to_owned())
            .collect()
    }

    fn matches(names: &[&str], candidate: &str) -> bool {
        let set: BTreeSet<String> = names.iter().map(|name| (*name).to_owned()).collect();
        RegexSet::new(patterns(&allowlist_toml(&set)))
            .unwrap()
            .is_match(candidate)
    }

    /// 名单只放行列出的名字；RegexSet 是子串匹配，正则必须锚定，点必须转义。
    #[test]
    fn the_allowlist_matches_exactly_the_listed_names() {
        let a = "abcdefghijklmnopqrstuvwx.wonton.internal";
        let b = "zyxwvutsrqponmlkjihgfedcb.wonton.internal";
        let a_suffix = "xxabcdefghijklmnopqrstuvwx.wonton.internal";
        assert!(matches(&[a, b], a));
        assert!(matches(&[a, b], b));
        assert!(
            !matches(&[a, b], a_suffix),
            "名字是别的名字的后缀也不能放行"
        );
        assert!(!matches(&[a, b], "other.example.org"));
    }

    /// 没有配对时名单谁也匹配不上：一个带下划线的名字，校验过的服务器名里不可能出现。
    #[test]
    fn the_empty_allowlist_matches_nothing() {
        assert!(!matches(&[], "abcdefghijklmnopqrstuvwx.wonton.internal"));
        assert!(!matches(&[], "anything.example.org"));
    }
}
