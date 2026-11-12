"""界面文字文件的路径：按文件主干名算出各平台的相对路径。

iOS 的表名来自文件名，拆出来的表放在 `strings/` 下，默认表 `Localizable.xcstrings` 留在原处。
Android 每个 `values` 目录、Fluent 每种语言目录下都按同样的主干名放一份。
"""

from __future__ import annotations

from typing import Iterator, List, Tuple

# 四种语言和各自的目录（ADR-0043）。
LANGUAGES = ("en", "zh-Hans", "zh-Hant", "ja")
ANDROID_DIRS = {
    "en": "values",
    "zh-Hans": "values-b+zh+Hans",
    "zh-Hant": "values-b+zh+Hant",
    "ja": "values-ja",
}
ANDROID_MODULES = (
    "apps/compose/shared/src/androidMain/res",
    "apps/compose/androidApp/src/main/res",
)

IOS_DIR = "apps/ios/Wonton"
IOS_STRINGS_DIR = f"{IOS_DIR}/strings"
FTL_DIR = "crates/wonton-client/locales"
FTL_BASE = "common.ftl"


def ios_path(stem: str) -> str:
    return f"{IOS_STRINGS_DIR}/{stem}.xcstrings" if stem else f"{IOS_DIR}/Localizable.xcstrings"


def android_path(module: str, lang: str, stem: str) -> str:
    name = f"strings_{stem}.xml" if stem else "strings.xml"
    return f"{module}/{ANDROID_DIRS[lang]}/{name}"


def android_paths(lang: str, stem: str) -> Iterator[str]:
    for module in ANDROID_MODULES:
        yield android_path(module, lang, stem)


def ftl_path(lang: str, stem: str) -> str:
    return f"{FTL_DIR}/{lang}/{stem}.ftl" if stem else f"{FTL_DIR}/{lang}/{FTL_BASE}"


def path_of(platform: str, lang: str, stem: str) -> List[str]:
    """某个平台上这个主干名对应的一份或几份文件。"""
    if platform == "ios":
        return [ios_path(stem)]
    if platform == "android":
        return list(android_paths(lang, stem))
    return [ftl_path(lang, stem)]


def all_paths(platform: str) -> List[str]:
    """放这类文件的目录，用来扫现有文件。"""
    if platform == "ios":
        return [IOS_STRINGS_DIR, IOS_DIR]
    if platform == "android":
        return [f"{module}/{ANDROID_DIRS[lang]}" for module in ANDROID_MODULES for lang in LANGUAGES]
    return [f"{FTL_DIR}/{lang}" for lang in LANGUAGES]


def file_kind(path: str) -> Tuple[str, str, str] | None:
    """路径换回（平台，语言，主干名）；不认识的返回 None。"""
    if path.startswith(f"{IOS_STRINGS_DIR}/") and path.endswith(".xcstrings"):
        return "ios", "", path.rsplit("/", 1)[-1][: -len(".xcstrings")]
    if path == f"{IOS_DIR}/Localizable.xcstrings":
        return "ios", "", ""
    for module in ANDROID_MODULES:
        for lang, dirname in ANDROID_DIRS.items():
            prefix = f"{module}/{dirname}/"
            if not path.startswith(prefix) or not path.endswith(".xml"):
                continue
            name = path[len(prefix) :]
            if name == "strings.xml":
                return "android", lang, ""
            if name.startswith("strings_") and name.endswith(".xml"):
                return "android", lang, name[len("strings_") : -len(".xml")]
    for lang in LANGUAGES:
        prefix = f"{FTL_DIR}/{lang}/"
        if path.startswith(prefix) and path.endswith(".ftl"):
            name = path[len(prefix) :]
            return "fluent", lang, "" if name == FTL_BASE else name[: -len(".ftl")]
    return None
