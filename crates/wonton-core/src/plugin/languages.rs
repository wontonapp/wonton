//! 按请求的 `Accept-Language` 挑插件文字的语言（ADR-0017 的“接口：App 和核心”）。
//! 协商规则在 [`wonton_l10n::choose_language`]，和核心目录、服务器文字用的是同一套。

use std::collections::BTreeMap;

use super::manifest::{LocalizedTags, LocalizedText};

/// 请求想要的语言，按偏好从高到低。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Languages(Vec<String>);

/// `Accept-Language` 里的语言标签，按偏好从高到低，保留原来的大小写：OIDC 的 `locale` 原样给，
/// 不收窄到核心支持的语言。`*` 和 q=0 的不要。
pub fn accept_language_tags(header: &str) -> Vec<String> {
    ranked(header).into_iter().map(|(_, tag)| tag).collect()
}

/// 按 q 值排的语言标签（q 值相同的保持原来的顺序）。
fn ranked(header: &str) -> Vec<(u32, String)> {
    let mut ranked: Vec<(u32, String)> = header
        .split(',')
        .filter_map(|item| {
            let mut parts = item.split(';').map(str::trim);
            let tag = parts.next().filter(|t| !t.is_empty() && *t != "*")?;
            let quality = parts
                .find_map(|p| p.strip_prefix("q="))
                .map_or(Some(1000), |q| {
                    q.parse::<f32>().ok().map(|q| (q * 1000.0) as u32)
                })?;
            (quality > 0).then(|| (quality, tag.to_owned()))
        })
        .collect();
    ranked.sort_by_key(|(quality, _)| std::cmp::Reverse(*quality));
    ranked
}

impl Languages {
    /// 解析 `Accept-Language`（如 `zh-Hans-CN, en;q=0.8`）。q 值相同的保持原来的顺序；q=0 的和 `*` 不要。
    pub fn from_header(header: Option<&str>) -> Self {
        Self(
            ranked(header.unwrap_or_default())
                .into_iter()
                .map(|(_, tag)| tag.to_ascii_lowercase())
                .collect(),
        )
    }

    pub(crate) fn choose<'t, T>(&self, text: &'t BTreeMap<String, T>) -> Option<&'t T> {
        let values: Vec<&'t T> = text.values().collect();
        let tags: Vec<&str> = text.keys().map(String::as_str).collect();
        wonton_l10n::choose_language(&self.0, &tags)
            .map(|index| values[index])
            .or_else(|| values.first().copied())
    }

    /// 请求想要的语言，按偏好从高到低。
    pub(crate) fn requested(&self) -> &[String] {
        &self.0
    }

    /// 从按语言组织的清单里挑一项，使用与插件文字相同的回退规则。
    pub fn pick_map<'t, T>(&self, text: &'t BTreeMap<String, T>) -> Option<&'t T> {
        self.choose(text)
    }

    /// 挑出一种语言的文字。清单保证至少有一种语言；空表给空串。
    pub fn pick<'t>(&self, text: &'t LocalizedText) -> &'t str {
        self.choose(text).map_or("", String::as_str)
    }

    pub fn pick_owned(&self, text: &LocalizedText) -> String {
        self.pick(text).to_owned()
    }

    pub fn pick_tags(&self, tags: &LocalizedTags) -> Vec<String> {
        self.choose(tags).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(pairs: &[(&str, &str)]) -> LocalizedText {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn picks_by_preference_then_primary_language_then_english_then_any() {
        let title = text(&[("zh", "相册"), ("en", "Photos"), ("zh-TW", "相簿")]);
        let pick = |header: &str| Languages::from_header(Some(header)).pick(&title);
        assert_eq!(
            pick("zh-Hans-CN, en"),
            "相册",
            "主语言相同的排在下一种语言前面"
        );
        assert_eq!(pick("zh-TW"), "相簿");
        assert_eq!(
            pick("en-GB;q=0.9, ja"),
            "Photos",
            "按 q 值排序，日文没有就看下一种"
        );
        assert_eq!(pick("fr"), "Photos", "都没有时用英文");
        assert_eq!(
            pick("en;q=0, fr"),
            "Photos",
            "q=0 表示不要这个偏好，回退还是英文"
        );
        assert_eq!(Languages::default().pick(&title), "Photos");
        let only_chinese = text(&[("zh", "相册")]);
        assert_eq!(
            Languages::from_header(Some("en")).pick(&only_chinese),
            "相册"
        );
    }

    /// 每种可用语言的文字就是它的标签，断言挑中的标签。
    fn picked(header: &str, available: &[&str]) -> String {
        let texts: LocalizedText = available
            .iter()
            .map(|tag| ((*tag).to_owned(), (*tag).to_owned()))
            .collect();
        Languages::from_header(Some(header)).pick_owned(&texts)
    }

    #[test]
    fn traditional_chinese_prefers_zh_hant_then_falls_back_to_zh() {
        assert_eq!(picked("zh-TW", &["zh", "zh-Hant", "en"]), "zh-Hant");
        assert_eq!(picked("zh-TW", &["zh", "en"]), "zh");
        assert_eq!(picked("zh-TW", &["zh-Hans", "zh-Hant"]), "zh-Hant");
    }

    #[test]
    fn hong_kong_and_macau_pick_zh_hant_when_available() {
        for header in ["zh-HK", "zh-MO"] {
            assert_eq!(
                picked(header, &["zh", "zh-Hant", "en"]),
                "zh-Hant",
                "{header}"
            );
        }
    }

    #[test]
    fn mainland_singapore_and_plain_zh_pick_simplified() {
        for header in ["zh-CN", "zh-SG", "zh"] {
            assert_eq!(picked(header, &["zh", "zh-Hant", "en"]), "zh", "{header}");
            assert_eq!(
                picked(header, &["zh-Hans", "zh-Hant", "en"]),
                "zh-Hans",
                "{header}"
            );
        }
    }

    #[test]
    fn unmatched_language_falls_back_to_english() {
        assert_eq!(picked("ja", &["zh", "en"]), "en");
        assert_eq!(picked("fr-FR, ja", &["ja", "en"]), "ja");
        assert_eq!(picked("ja", &["zh", "en-GB"]), "en-GB");
    }

    #[test]
    fn without_english_or_match_picks_some_available_value() {
        let value = picked("fr", &["ja", "zh"]);
        assert!(["ja", "zh"].contains(&value.as_str()), "{value}");
    }

    #[test]
    fn empty_header_picks_english() {
        assert_eq!(picked("", &["zh", "en", "ja"]), "en");
    }

    #[test]
    fn tags_match_case_insensitively_and_keep_the_map_key() {
        assert_eq!(picked("ZH-tw", &["zh-hant", "en"]), "zh-hant");
        assert_eq!(picked("EN", &["zh", "En"]), "En");
    }

    #[test]
    fn underscore_keys_still_match() {
        assert_eq!(picked("zh-TW", &["zh_Hant", "en"]), "zh_Hant");
    }

    #[test]
    fn tags_keep_their_case_and_weight_order() {
        assert_eq!(
            accept_language_tags("zh-Hant-TW, en;q=0.8, *;q=0.5, ja;q=0"),
            ["zh-Hant-TW", "en"],
            "原样给，按 q 排序，`*` 和 q=0 不要"
        );
        assert_eq!(accept_language_tags("en;q=0.4, ja"), ["ja", "en"]);
        assert!(accept_language_tags("").is_empty());
    }
}
