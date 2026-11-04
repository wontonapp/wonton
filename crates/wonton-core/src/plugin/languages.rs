//! 按请求的 `Accept-Language` 挑插件文字的语言（ADR-0017 的“接口：App 和核心”）：对每个想要的语言，先找完全一样的
//! 语言标签，再找主语言相同的；都没有时用英文，再没有就用任意一种。

use std::collections::BTreeMap;

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
        let find = |wanted: &str| {
            text.iter()
                .find(|(lang, _)| lang.eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value)
        };
        let same_primary = |wanted: &str| {
            let base = primary(wanted);
            find(base).or_else(|| {
                text.iter()
                    .find(|(lang, _)| primary(lang).eq_ignore_ascii_case(base))
                    .map(|(_, value)| value)
            })
        };
        self.0
            .iter()
            .find_map(|wanted| find(wanted).or_else(|| same_primary(wanted)))
            .or_else(|| same_primary("en"))
            .or_else(|| text.values().next())
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

fn primary(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or(tag)
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
}
