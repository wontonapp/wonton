//! 桥发进聊天、用户看得到的文字（ADR-0043）：按房间主人的语言从 Fluent 目录取。
//! 各语言由 `build.rs` 把 `locales/<语言>/*.ftl` 拼成一份；日志（`tracing`）不翻译。

use std::sync::LazyLock;

use wonton_l10n::{Arg, Catalog, Lang};

pub(crate) static RESOURCES: &[(&str, &str)] = &[
    ("en", include_str!(concat!(env!("OUT_DIR"), "/en.ftl"))),
    (
        "zh-Hans",
        include_str!(concat!(env!("OUT_DIR"), "/zh-Hans.ftl")),
    ),
    (
        "zh-Hant",
        include_str!(concat!(env!("OUT_DIR"), "/zh-Hant.ftl")),
    ),
    ("ja", include_str!(concat!(env!("OUT_DIR"), "/ja.ftl"))),
];

pub(crate) static CATALOG: LazyLock<Catalog> = LazyLock::new(|| Catalog::new(RESOURCES));

/// 核心的默认语言：目录的第一个资源（英文）。
pub(crate) fn default_lang() -> Lang {
    let none: [&str; 0] = [];
    CATALOG.negotiate(&none)
}

/// 日志里的错误文字用的语言：日志一直写中文，不跟房间主人走。
pub(crate) fn log_lang() -> Lang {
    CATALOG.negotiate(&["zh-Hans"])
}

/// 测试里的固定语言：现有测试的断言都按简体中文写。
#[cfg(test)]
pub(crate) fn zh() -> Lang {
    log_lang()
}

/// 按语言写一条消息。
pub(crate) fn text(lang: Lang, id: &str, args: &[(&str, Arg<'_>)]) -> String {
    CATALOG.format(lang, id, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_l10n::check_catalog;

    #[test]
    fn four_languages_have_the_same_messages_and_variables() {
        check_catalog(RESOURCES).expect("catalog inconsistent");
    }

    #[test]
    fn requests_negotiate_the_four_languages() {
        for (requested, tag) in [
            ("en", "en"),
            ("en-GB", "en"),
            ("fr-FR", "en"),
            ("zh-TW", "zh-Hant"),
            ("zh-HK", "zh-Hant"),
            ("zh-CN", "zh-Hans"),
            ("ja-JP", "ja"),
            ("", "en"),
        ] {
            assert_eq!(
                CATALOG.tag(CATALOG.negotiate(&[requested])),
                tag,
                "{requested}"
            );
        }
    }
}
