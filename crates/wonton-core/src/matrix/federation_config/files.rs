//! 联邦两份文件的生成（ADR-0050）：`fed/config.toml` 的名单正则和 `fed/hosts`。纯函数，运行时在 [super]。

use std::collections::BTreeSet;
use std::net::IpAddr;

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

/// hosts 文件的内容：回环两行加每个配对过的服务器名指向核心的固定地址。
pub(crate) fn hosts(names: &BTreeSet<String>, core_addr: IpAddr) -> String {
    let mut out = String::from(
        "# 核心生成并原地重写，别手改（ADR-0050）。\n\
         # Tuwunel 只在启动时读它：内容变了核心会重启 Tuwunel。\n\
         127.0.0.1 localhost\n\
         ::1 localhost\n",
    );
    for name in names {
        out.push_str(&format!("{core_addr} {name}\n"));
    }
    out
}

/// hosts 里记下的配对名字：指向核心固定地址的行。
pub(crate) fn names_in_hosts(text: &str, core_addr: IpAddr) -> BTreeSet<String> {
    let prefix = core_addr.to_string();
    text.lines()
        .filter(|line| line.starts_with(&prefix))
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_owned))
        .collect()
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
    use std::net::{IpAddr, Ipv4Addr};

    use regex::RegexSet;

    use super::{allowlist_toml, hosts, names_in_hosts};

    const ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 212, 0, 10));

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

    #[test]
    fn the_hosts_file_points_every_name_at_the_core() {
        let mut names = BTreeSet::new();
        names.insert("aa.wonton.internal".to_owned());
        names.insert("bb.wonton.internal".to_owned());
        let file = hosts(&names, ADDR);
        assert!(file.contains("127.0.0.1 localhost\n"), "{file}");
        assert_eq!(file.matches(&ADDR.to_string()).count(), 2, "{file}");
        assert_eq!(names_in_hosts(&file, ADDR), names, "{file}");
        assert_eq!(
            names_in_hosts("127.0.0.1 localhost\n", ADDR),
            BTreeSet::new()
        );
    }
}
