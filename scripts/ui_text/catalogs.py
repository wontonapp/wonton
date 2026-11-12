"""字符串目录的完整性（规则三）：iOS 的 .xcstrings、Android 的 strings.xml、Fluent 的 .ftl。

四种语言（en、zh-Hans、zh-Hant、ja）的键要一样；每个键每种语言都有译文；占位符的种类和个数要一样。
占位符比较时忽略位置（%2$@ 和 %@ 算同一种），每种占位符取各个形式里的最大个数
（复数的 one 和 other 都带 %lld，中文只有 other，两边都是一个 %lld）。

文字按功能分文件（ADR-0043）：键的前缀决定它在哪个文件，同一个键不能出现在两个文件里。
分组表见 `groups.py`，文件路径见 `paths.py`。
"""

from __future__ import annotations

import json
import re
import xml.etree.ElementTree as ET
from collections import Counter
from typing import Dict, List, Optional, Tuple

from . import groups, paths  # noqa: E402
from .source import Problem, is_cjk

XC_LANGS = ("en", "zh-Hans", "zh-Hant", "ja")
# Android 的 values 目录和 BCP 47 语言的对应。
ANDROID_DIRS = {
    "en": "values",
    "zh-Hans": "values-b+zh+Hans",
    "zh-Hant": "values-b+zh+Hant",
    "ja": "values-ja",
}
ANDROID_TAGS = ("string", "plurals", "string-array")
PLURAL_REQUIRED = {"en": ("one", "other")}  # 其他语言只要求 other

PLACEHOLDER_RE = re.compile(r"%%|%(?:\d+\$)?[-+#0]*\d*(?:\.\d+)?((?:hh|h|ll|l|z|j|t|L)?[@a-zA-Z])")


def placeholder_signature(texts: List[str]) -> frozenset:
    """各个文本的占位符种类，每种取最大个数。"""
    best: Counter = Counter()
    for text in texts:
        counts = Counter(m.group(1) for m in PLACEHOLDER_RE.finditer(text) if m.group(0) != "%%")
        for kind, n in counts.items():
            best[kind] = max(best[kind], n)
    return frozenset(best.items())


def _line_of(text: str, needle: str) -> int:
    index = text.find(needle)
    return 1 if index < 0 else text.count("\n", 0, index) + 1


# ---- iOS 字符串目录 ----

def _string_units(node) -> List[dict]:
    """localization 里所有 stringUnit：普通条目是一个，变体（复数、设备）是多个。"""
    if not isinstance(node, dict):
        return []
    if isinstance(node.get("stringUnit"), dict):
        return [node["stringUnit"]]
    units: List[dict] = []
    for child in node.values():
        units.extend(_string_units(child))
    return units


def _xc_entry_problems(key: str, entry: dict, line: int) -> List[Problem]:
    problems: List[Problem] = []
    localizations = entry.get("localizations") or {}
    signatures: Dict[str, frozenset] = {}
    for lang in XC_LANGS:
        units = _string_units(localizations.get(lang))
        if not units:
            problems.append((line, f"「{key}」缺少 {lang} 翻译"))
            continue
        if any(u.get("state") != "translated" or not u.get("value") for u in units):
            problems.append((line, f"「{key}」的 {lang} 翻译还没完成（state 要是 translated，value 不能为空）"))
        signatures[lang] = placeholder_signature([str(u.get("value", "")) for u in units])
    source = signatures.get("en")
    for lang, sig in signatures.items():
        if source is not None and sig != source:
            problems.append((line, f"「{key}」的 {lang} 占位符和 en 不一致"))
    return problems


def check_xcstrings(text: str) -> List[Problem]:
    try:
        data = json.loads(text)
    except ValueError as err:
        return [(1, f"字符串目录不是合法 JSON：{err}")]
    problems: List[Problem] = []
    if data.get("sourceLanguage") != "en":
        problems.append((1, "sourceLanguage 必须是 en"))
    for key, entry in (data.get("strings") or {}).items():
        if not isinstance(entry, dict) or entry.get("shouldTranslate") is False:
            continue
        line = _line_of(text, json.dumps(key, ensure_ascii=False))
        problems += _xc_entry_problems(key, entry, line)
    return problems


def xcstring_keys(text: str) -> List[Tuple[str, int]]:
    """目录里的（键，行号）。"""
    data = json.loads(text)
    return [
        (key, _line_of(text, json.dumps(key, ensure_ascii=False)))
        for key in (data.get("strings") or {})
    ]


FTL_KEY_RE = re.compile(r"^([A-Za-z0-9-]+)\s*=", re.MULTILINE)


def ftl_keys(text: str) -> List[Tuple[str, int]]:
    """Fluent 文件里的（键，行号）。"""
    return [(match.group(1), text.count("\n", 0, match.start()) + 1) for match in FTL_KEY_RE.finditer(text)]


def android_keys(text: str) -> List[Tuple[str, int]]:
    return [(name, _line_of(text, f'name="{name}"')) for name in _parse_android(text)]


def layout_problems(found: List[Tuple[str, List[Tuple[str, int]]]], platform: str) -> Dict[str, List[Problem]]:
    """键有没有放进它该在的文件、有没有在别的文件里也有一份（ADR-0043）。

    重复按语言比：Android 再分模块（两个模块的键互不覆盖），iOS 一个目录里就是四种语言。
    """
    result: Dict[str, List[Problem]] = {}
    seen: Dict[Tuple[str, str, str], str] = {}
    for path, keys in found:
        kind = paths.file_kind(path)
        stem = kind[2]
        scope = (path.rsplit("/", 2)[0] if platform == "android" else "", kind[1])
        for key, line in keys:
            target = groups.resolve(key, platform)[1]
            if target != stem:
                hint = groups.matching_prefixes(target)
                where = f"（{groups.GROUP_OF[hint[0]]} 组）" if hint else ""
                problems = result.setdefault(path, [])
                problems.append((line, f"「{key}」按前缀该放进 {groups.file_name(target, platform)}{where}"))
            if (scope, key) in seen:
                result.setdefault(path, []).append((line, f"「{key}」和 {seen[(scope, key)]} 里重复"))
            else:
                seen[(scope, key)] = path
    return result


def ftl_missing(stem: str, texts: Dict[str, Optional[str]]) -> Dict[str, List[Problem]]:
    """每种语言的这份文件的键要和 en 一样。缺文件报给 en 的文件，多余的键报在自己文件上。"""
    en_path = paths.ftl_path("en", stem)
    en_keys = [key for key, _ in ftl_keys(texts["en"] or "")]
    result: Dict[str, List[Problem]] = {}
    for lang, text in texts.items():
        path = paths.ftl_path(lang, stem)
        if text is None:
            result.setdefault(en_path, []).append((1, f"缺少 {path}（{lang} 的译文）"))
            continue
        names = ftl_keys(text)
        for key in en_keys:
            if key not in dict(names):
                result.setdefault(path, []).append((1, f"缺少「{key}」的 {lang} 译文"))
        for key, line in names:
            if key not in en_keys:
                result.setdefault(path, []).append((line, f"「{key}」在 en 里没有，{lang} 不能单独有"))
    return result


# ---- Android strings.xml ----

def _android_items(elem: ET.Element) -> Tuple[List[str], Dict[str, str]]:
    """返回 (各个文本, {复数形式: 文本})。"""
    if elem.tag == "plurals":
        forms = {item.get("quantity", ""): "".join(item.itertext()) for item in elem.findall("item")}
        return list(forms.values()), forms
    if elem.tag == "string-array":
        return ["".join(item.itertext()) for item in elem.findall("item")], {}
    return ["".join(elem.itertext())], {}


def _parse_android(text: str) -> Dict[str, Tuple[ET.Element, bool]]:
    """{name: (元素, 是否可翻译)}，只收 string、plurals 和 string-array。"""
    root = ET.fromstring(text.encode("utf-8"))
    names: Dict[str, Tuple[ET.Element, bool]] = {}
    for elem in root:
        name = elem.get("name")
        if elem.tag in ANDROID_TAGS and name is not None:
            names[name] = (elem, elem.get("translatable") != "false")
    return names


def _plural_problems(lang: str, names: Dict[str, Tuple[ET.Element, bool]], text: str) -> List[Problem]:
    required = PLURAL_REQUIRED.get(lang, ("other",))
    problems: List[Problem] = []
    for name, (elem, translatable) in names.items():
        if not translatable or elem.tag != "plurals":
            continue
        _, forms = _android_items(elem)
        missing = [q for q in required if q not in forms]
        if missing:
            line = _line_of(text, f'name="{name}"')
            problems.append((line, f"「{name}」的 {lang} 缺少复数形式 {'、'.join(missing)}"))
    return problems


def check_android_values(lang_texts: Dict[str, str]) -> Dict[str, List[Problem]]:
    """lang_texts：{语言: strings.xml 内容}，en 一定在。返回 {语言: 问题}。"""
    result: Dict[str, List[Problem]] = {lang: [] for lang in lang_texts}
    en_text = lang_texts["en"]
    en = _parse_android(en_text)
    result["en"] += _plural_problems("en", en, en_text)
    # 基础语言是 en：基础目录里出现中文、日文或全角字符，多半是两种语言放反了文件。
    for name, (elem, translatable) in sorted(en.items()):
        if not translatable:
            continue
        if any(is_cjk(ch) for value in _android_items(elem)[0] for ch in value):
            result["en"].append(
                (
                    _line_of(en_text, f'name="{name}"'),
                    f"「{name}」在基础语言（en）的 values/strings.xml 里，却有中文、日文或全角字符，"
                    "像是放反了文件：values 里写英文，中文和日文放进 values-b+zh+Hans、"
                    'values-b+zh+Hant、values-ja；语言的自称这类不翻译的词条加 translatable="false"',
                ),
            )
    en_names = {n for n, (_, translatable) in en.items() if translatable}
    for lang, text in lang_texts.items():
        if lang == "en":
            continue
        names = _parse_android(text)
        problems = result[lang]
        for name, (_, translatable) in names.items():
            if not translatable:
                problems.append((_line_of(text, f'name="{name}"'), f"「{name}」标了 translatable=\"false\"，只能放在 values/ 里"))
        translatable_here = {n for n, (_, t) in names.items() if t}
        for name in sorted(en_names - translatable_here):
            problems.append((1, f"缺少「{name}」的 {lang} 翻译"))
        for name in sorted(translatable_here - en_names):
            problems.append((_line_of(text, f'name="{name}"'), f"「{name}」在 values/ 里没有，{lang} 不能单独有"))
        for name in sorted(en_names & translatable_here):
            en_texts = _android_items(en[name][0])[0]
            texts = _android_items(names[name][0])[0]
            if placeholder_signature(texts) != placeholder_signature(en_texts):
                problems.append((_line_of(text, f'name="{name}"'), f"「{name}」的 {lang} 占位符和 en 不一致"))
        problems += _plural_problems(lang, names, text)
    return result


def android_problems(en_text: str, others: Dict[str, Optional[str]]) -> Dict[str, List[Problem]]:
    """others：{语言: 文件内容或 None（文件缺失）}。缺失的文件在结果里报给 en 的文件。"""
    if not en_text.strip():
        return {"en": [(1, "英文的 values 里没有这份文件")]}
    lang_texts = {"en": en_text}
    missing: List[str] = []
    for lang, text in others.items():
        if text is None:
            missing.append(lang)
        else:
            lang_texts[lang] = text
    result = check_android_values(lang_texts)
    result.setdefault("en", [])
    for lang in missing:
        result["en"].append((1, f"缺少 {ANDROID_DIRS[lang]}/strings.xml（{lang} 的译文）"))
    return result
