//! 核心发给 App 的文字（ADR-0043）：插件的权限说明、安装确认卡片里的提示这些“只有服务器知道”的话。
//! 按请求的 `Accept-Language`（Agent 发起的卡片按收件人的语言，没有记录时按主人的）协商出语言；App 拿到的直接是成品。

use std::sync::LazyLock;

use wonton_l10n::{Arg, Catalog, Lang};

use crate::plugin::languages::Languages;

/// 各语言由 `build.rs` 把 `locales/<语言>/*.ftl` 拼成一份。
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

/// 这个请求的文字用的语言。
pub(crate) fn lang(languages: &Languages) -> Lang {
    CATALOG.negotiate(languages.requested())
}

/// 协商出的语言标签（`zh-Hans`、`zh-Hant`、`ja`、`en`），网页的 `lang` 属性用它。
pub(crate) fn lang_tag(languages: &Languages) -> &'static str {
    CATALOG.tag(lang(languages))
}

/// 按请求的语言写一条消息。
pub(crate) fn text(languages: &Languages, id: &str, args: &[(&str, Arg<'_>)]) -> String {
    CATALOG.format(lang(languages), id, args)
}

/// 列举数据项（镜像仓库这些）：分隔符跟着语言走，Fluent 本身没有列表格式化器。
pub(crate) fn list(languages: &Languages, items: &[impl AsRef<str>]) -> String {
    let separator = text(languages, "list-separator", &[]);
    items
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(&separator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::languages::Languages;
    use wonton_l10n::check_catalog;

    #[test]
    fn four_languages_have_the_same_messages_and_variables() {
        check_catalog(RESOURCES).expect("catalog inconsistent");
    }

    #[test]
    fn requests_negotiate_the_four_languages() {
        for (header, tag) in [
            ("ja", "ja"),
            ("fr-FR", "en"),
            ("zh-TW", "zh-Hant"),
            ("zh-CN", "zh-Hans"),
            ("", "en"),
        ] {
            let lang = lang(&Languages::from_header(Some(header)));
            let expected = CATALOG.negotiate(&[tag]);
            assert_eq!(
                CATALOG.format(lang, "permission-models", &[]),
                CATALOG.format(expected, "permission-models", &[]),
                "{header}"
            );
        }
    }
}
