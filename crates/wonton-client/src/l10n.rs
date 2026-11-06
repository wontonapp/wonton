use std::sync::{LazyLock, RwLock};

use wonton_l10n::{Arg, Catalog, Lang};

pub(crate) const RESOURCES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.ftl")),
    ("zh-Hans", include_str!("../locales/zh-Hans.ftl")),
    ("zh-Hant", include_str!("../locales/zh-Hant.ftl")),
    ("ja", include_str!("../locales/ja.ftl")),
];

pub(crate) static CATALOG: LazyLock<Catalog> = LazyLock::new(|| Catalog::new(RESOURCES));
static LANGUAGE: LazyLock<RwLock<Lang>> = LazyLock::new(|| RwLock::new(CATALOG.negotiate(&["en"])));

pub(crate) fn init() {
    wonton_protocol::set_core_error_formatter(|id, reason| text(id, &[("reason", reason.into())]));
}

pub(crate) fn set_languages(languages: &[String]) {
    // 后续项目是系统偏好，不可覆盖第一项已由壳选定的 App 语言。
    let requested = &languages[..languages.len().min(1)];
    *LANGUAGE.write().expect("client language lock poisoned") = CATALOG.negotiate(requested);
}

pub(crate) fn language() -> Lang {
    *LANGUAGE.read().expect("client language lock poisoned")
}

pub(crate) fn text(id: &str, args: &[(&str, Arg<'_>)]) -> String {
    CATALOG.format(language(), id, args)
}

#[cfg(test)]
mod tests;
