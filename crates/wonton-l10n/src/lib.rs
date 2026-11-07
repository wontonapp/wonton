//! 调用方持有自己的资源与语言状态；语言句柄不能跨目录使用。
//!
//! ```
//! use wonton_l10n::{Catalog, check_catalog, l10n};
//! static RESOURCES: &[(&str, &str)] = &[("en", "download-failed = HTTP { $status }\n")];
//! check_catalog(RESOURCES).unwrap();
//! let catalog = Catalog::new(RESOURCES);
//! let lang = catalog.negotiate(&["en"]);
//! assert_eq!(l10n!(catalog, lang, "download-failed", status = 503), "HTTP 503");
//! ```

use std::cmp::Reverse;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use fluent_langneg::{NegotiationStrategy, negotiate_languages};
use unic_langid::LanguageIdentifier;

mod check;

pub use check::check_catalog;

/// Fluent 的借用字符串、拥有的字符串或数字参数。
pub type Arg<'a> = fluent_bundle::FluentValue<'a>;

/// 只能用于生成它的目录。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lang(usize);

/// 按请求的语言偏好（第一个最想要）从一组语言标签里挑出一个的下标，供插件文字和 Fluent 目录共用。
///
/// 请求的标签先按 CLDR 的可能子标签补全（`zh-TW` 补成 `zh-Hant-TW`，需要 unic-langid 的
/// `likelysubtags` 特性），再按 Lookup 策略匹配；更具体的可用标签优先匹配（`zh-Hant` 在 `zh` 前）。
/// 都没有时用 `available` 里最不具体的 `en`，连 `en` 都没有时返回 `None`。
/// 解析不了的标签不参与协商；`_` 当 `-` 看，清单里写过 `zh_TW` 的插件不能因此选不中。
pub fn choose_language(
    requested: &[impl AsRef<str>],
    available: &[impl AsRef<str>],
) -> Option<usize> {
    let mut candidates: Vec<Entry> = available
        .iter()
        .enumerate()
        .filter_map(|(index, tag)| {
            Some(Entry {
                id: langid(tag.as_ref())?,
                index,
            })
        })
        .collect();
    // 库取列表里第一个匹配的语言。`zh` 也能匹配补全后的 `zh-Hant-TW`，所以更具体的要排在前面，
    // 否则 `zh` 总是先被选中。
    candidates.sort_by_key(|entry| Reverse(specificity(&entry.id)));
    let requested: Vec<LanguageIdentifier> = requested
        .iter()
        .filter_map(|tag| langid(tag.as_ref()))
        .map(|mut id| {
            id.maximize();
            id
        })
        .collect();
    let default = candidates
        .iter()
        .filter(|entry| entry.id.language.as_str() == "en")
        .min_by_key(|entry| specificity(&entry.id));
    negotiate_languages(
        &requested,
        &candidates,
        default,
        NegotiationStrategy::Lookup,
    )
    .first()
    .map(|entry| entry.index)
}

/// 可用语言里的一个标签：解析后的语言，和它在调用方列表里的位置（挑中后用它取回原来的文字）。
#[derive(PartialEq)]
struct Entry {
    id: LanguageIdentifier,
    index: usize,
}

impl AsRef<LanguageIdentifier> for Entry {
    fn as_ref(&self) -> &LanguageIdentifier {
        &self.id
    }
}

/// 解析语言标签，`_` 当 `-` 看。
fn langid(tag: &str) -> Option<LanguageIdentifier> {
    LanguageIdentifier::from_bytes(tag.replace('_', "-").as_bytes()).ok()
}

/// 标签越全越具体：文字系统、地区、变体都要比。
fn specificity(id: &LanguageIdentifier) -> usize {
    usize::from(id.script.is_some())
        + usize::from(id.region.is_some())
        + usize::from(id.variants().len() > 0)
}

pub struct Catalog {
    tags: Vec<&'static str>,
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Catalog {
    /// 基础语言必须排第一且为 `en`；资源随二进制发布，错误由 CI 提前检查。
    pub fn new(resources: &'static [(&'static str, &'static str)]) -> Self {
        assert_eq!(resources.first().map(|(tag, _)| *tag), Some("en"));
        let mut tags = Vec::with_capacity(resources.len());
        let mut bundles = Vec::with_capacity(resources.len());
        for (tag, text) in resources {
            let language: LanguageIdentifier = tag.parse().expect("invalid catalog language tag");
            assert!(!tags.contains(tag), "duplicate catalog language: {tag}");
            let resource = FluentResource::try_new((*text).to_owned())
                .unwrap_or_else(|(_, errors)| panic!("invalid Fluent resource {tag}: {errors:?}"));
            let mut bundle = FluentBundle::new_concurrent(vec![language.clone()]);
            // 仅支持从左到右的语言，避免隔离符混入复制的详情和日志。
            bundle.set_use_isolating(false);
            bundle
                .add_builtins()
                .expect("Fluent builtins must be unique");
            bundle
                .add_resource(resource)
                .unwrap_or_else(|errors| panic!("duplicate Fluent entries {tag}: {errors:?}"));
            tags.push(tag);
            bundles.push(bundle);
        }
        Self { tags, bundles }
    }

    pub fn negotiate(&self, requested: &[impl AsRef<str>]) -> Lang {
        // `new` 保证第一种语言是 `en`，协商一定能落到它。
        Lang(choose_language(requested, &self.tags).expect("catalog must include en"))
    }

    /// 挑中的语言的标签（`zh-Hans`、`ja`…），网页的 `lang` 属性用它。
    pub fn tag(&self, lang: Lang) -> &'static str {
        self.tags[lang.0]
    }

    pub fn format(&self, lang: Lang, id: &str, args: &[(&str, Arg<'_>)]) -> String {
        let bundle = &self.bundles[lang.0];
        let (bundle, pattern) = match bundle.get_message(id).and_then(|message| message.value()) {
            Some(pattern) => (bundle, pattern),
            None => {
                let base = &self.bundles[0];
                let Some(pattern) = base.get_message(id).and_then(|message| message.value()) else {
                    return id.to_owned();
                };
                (base, pattern)
            }
        };
        let mut fluent_args = FluentArgs::with_capacity(args.len());
        for (name, value) in args {
            let value = match value {
                Arg::String(text) => Arg::from(text.as_ref()),
                other => other.clone(),
            };
            fluent_args.set(*name, value);
        }
        let mut errors = Vec::new();
        let text = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
        assert!(
            errors.is_empty(),
            "cannot format Fluent message {id}: {errors:?}"
        );
        text.into_owned()
    }
}

#[macro_export]
macro_rules! l10n {
    ($catalog:expr, $lang:expr, $id:expr $(, $name:ident = $value:expr)* $(,)?) => {
        ($catalog).format($lang, $id, &[$((stringify!($name), $crate::Arg::from($value))),*])
    };
}

#[cfg(test)]
mod tests;
