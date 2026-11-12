//! 按请求的 `Accept-Language` 挑插件文字的语言（ADR-0017 的“接口：App 和核心”）：用 fluent-langneg 的 Lookup 策略协商。
//! 请求的语言先补全成 CLDR 的可能子标签（`zh-TW` 补成 `zh-Hant-TW`），再和可用的语言比较；都没有时用英文，再没有就用任意一种。

use std::cmp::Reverse;
use std::collections::BTreeMap;

use fluent_langneg::{LanguageIdentifier, NegotiationStrategy, negotiate_languages};
use icu_locid_transform::LocaleExpander;

use super::manifest::{LocalizedTags, LocalizedText};

/// 请求想要的语言，按偏好从高到低。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Languages(Vec<String>);

impl Languages {
    /// 解析 `Accept-Language`（如 `zh-Hans-CN, en;q=0.8`）。q 值相同的保持原来的顺序；q=0 的和 `*` 不要。
    pub fn from_header(header: Option<&str>) -> Self {
        let mut ranked: Vec<(u32, String)> = header
            .unwrap_or_default()
            .split(',')
            .filter_map(|item| {
                let mut parts = item.split(';').map(str::trim);
                let tag = parts.next().filter(|t| !t.is_empty() && *t != "*")?;
                let quality = parts
                    .find_map(|p| p.strip_prefix("q="))
                    .map_or(Some(1000), |q| {
                        q.parse::<f32>().ok().map(|q| (q * 1000.0) as u32)
                    })?;
                (quality > 0).then(|| (quality, tag.to_ascii_lowercase()))
            })
            .collect();
        ranked.sort_by_key(|(quality, _)| std::cmp::Reverse(*quality));
        Self(ranked.into_iter().map(|(_, tag)| tag).collect())
    }

    pub(crate) fn choose<'t, T>(&self, text: &'t BTreeMap<String, T>) -> Option<&'t T> {
        let values: Vec<&'t T> = text.values().collect();
        let mut available: Vec<Tag> = text
            .keys()
            .enumerate()
            .filter_map(|(index, key)| {
                Some(Tag {
                    id: langid(key)?,
                    index,
                })
            })
            .collect();
        // 库取列表里第一个匹配的语言。`zh` 也能匹配 `zh-Hant-TW`，所以更具体的要排在前面，否则 `zh` 总是先被选中。
        available.sort_by_key(|tag| Reverse(specificity(&tag.id)));
        // 库在补全请求的语言之前就按前缀比较，不补全的话 `zh-TW` 会先匹配到 `zh`，`zh-Hant` 永远轮不到。
        let expander = LocaleExpander::new();
        let requested: Vec<LanguageIdentifier> = self
            .0
            .iter()
            .filter_map(|tag| langid(tag))
            .map(|mut id| {
                expander.maximize(&mut id);
                id
            })
            .collect();
        let default = available
            .iter()
            .filter(|tag| tag.id.language.as_str() == "en")
            .min_by_key(|tag| specificity(&tag.id));
        negotiate_languages(&requested, &available, default, NegotiationStrategy::Lookup)
            .first()
            .map(|tag| values[tag.index])
            .or_else(|| values.first().copied())
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

/// 清单里的一个语言：解析后的标签，和它在清单里的位置（挑中后用它取回原来的文字）。
#[derive(Debug, PartialEq)]
struct Tag {
    id: LanguageIdentifier,
    index: usize,
}

impl AsRef<LanguageIdentifier> for Tag {
    fn as_ref(&self) -> &LanguageIdentifier {
        &self.id
    }
}

/// 解析不了的标签不参与协商：请求的话跳过，清单里的只能靠最后的“任意一种”挑到。`_` 当 `-` 看：清单里写过 `zh_TW` 的插件不能因此选不中。
fn langid(tag: &str) -> Option<LanguageIdentifier> {
    LanguageIdentifier::try_from_bytes(tag.replace('_', "-").as_bytes()).ok()
}

fn specificity(id: &LanguageIdentifier) -> usize {
    usize::from(id.script.is_some())
        + usize::from(id.region.is_some())
        + usize::from(!id.variants.is_empty())
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
}
