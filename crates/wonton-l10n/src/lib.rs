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

pub struct Catalog {
    languages: Vec<LanguageIdentifier>,
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Catalog {
    /// 基础语言必须排第一且为 `en`；资源随二进制发布，错误由 CI 提前检查。
    pub fn new(resources: &'static [(&'static str, &'static str)]) -> Self {
        assert_eq!(resources.first().map(|(tag, _)| *tag), Some("en"));
        let mut languages = Vec::with_capacity(resources.len());
        let mut bundles = Vec::with_capacity(resources.len());
        for (tag, text) in resources {
            let language: LanguageIdentifier = tag.parse().expect("invalid catalog language tag");
            assert!(
                !languages.contains(&language),
                "duplicate catalog language: {tag}"
            );
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
            languages.push(language);
            bundles.push(bundle);
        }
        Self { languages, bundles }
    }

    pub fn negotiate(&self, requested: &[impl AsRef<str>]) -> Lang {
        let requested: Vec<LanguageIdentifier> = requested
            .iter()
            .filter_map(|tag| tag.as_ref().parse().ok())
            .collect();
        let negotiated = negotiate_languages(
            &requested,
            &self.languages,
            self.languages.first(),
            NegotiationStrategy::Lookup,
        );
        Lang(
            self.languages
                .iter()
                .position(|language| language == negotiated[0])
                .unwrap(),
        )
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
