"""界面文字文件的分组（ADR-0043）：键的前缀决定它落在哪个文件。

同一个键前缀只能有一个文件；不在这里的前缀只能进 common 的基准文件。文件主干名就是分组，
 знач是空串的是 common 的基准文件（iOS 的 `Localizable.xcstrings`、Android 的 `strings.xml`、
Fluent 的 `<语言>.ftl`），其余组用组名（`chat.xcstrings`、`strings_chat.xml`、`chat.ftl`）。

一个大前缀装不进 800 行时用尾部编号拆开（`message1`、`message2`…）：SAS 表情（`verification.emoji.0`）
按编号对份数取模——壳里按序号动态取表，两边必须同一套算法；其余键落哪份由 `crc32(键) % 份数` 决定。
分法只看键本身，插入新键不会挤走别的键，也不会都挤进同一份。
"""

from __future__ import annotations

import re
import zlib
from typing import Dict, Tuple

# 前缀 → 文件主干。空串是 common 的基准文件。
PREFIXES: Dict[str, str] = {
    # chat
    "chat": "chat",
    "chatbackup": "chatBackup",
    "chatstatus": "chat",
    "timeline": "chat",
    "message": "message",
    "room": "chat",
    "bot": "chat",
    "botprofile": "botProfile",
    "transfer": "transfer",
    "composer": "composer",
    "panel": "panel",
    "confirmation": "chat",
    "confirmationcard": "confirmationCard",
    # onboarding
    "onboarding": "onboarding",
    "pairing": "pairing",
    "install": "install",
    "home": "onboarding",
    "homestatus": "homeStatus",
    "launcher": "launcher",
    # plugins
    "plugin": "plugin",
    "pluginsources": "pluginSources",
    "store": "store",
    "registry": "plugins",
    "registrymirror": "registryMirror",
    "tool": "plugins",
    "toolpolicy": "toolPolicy",
    "tab": "tab",
    "tabs": "plugins",
    "files": "plugins",
    "file": "file",
    "viewer": "viewer",
    "photo": "photo",
    "preview": "plugins",
    "secret": "plugins",
    "sidebar": "sidebar",
    "sessions": "plugins",
    "scope": "plugins",
    "takeover": "takeover",
    "port": "plugins",
    "mapping": "plugins",
    "portmapping": "plugins",
    # peers
    "peers": "peers",
    "friends": "friends",
    "invite": "invite",
    "servers": "servers",
    "connection": "connection",
    "verification": "verification",
    "verification.emoji": "verificationEmoji",
    "lanclient": "peers",
    "lanwebui": "peers",
    "qr": "peers",
    "qrscanner": "peers",
    "camera": "peers",
    "ownerprofile": "peers",
    "keepalive": "peers",
    "share": "share",
    "contacts": "contacts",
    "handoff": "handoff",
    "matrixlogin": "matrixlogin",
    # settings
    "settings": "settings",
    "appearance": "appearance",
    "egress": "egress",
    "notification": "settings",
    "push": "settings",
    "shortcuts": "shortcuts",
    "shortcut": "settings",
    # common
    "common": "common",
    "problem": "problem",
    "core": "common",
    "coreupgrade": "coreUpgrade",
    "design": "design",
    "app": "",
    "language": "",
    "startup": "",
    "page": "",
    "pin": "",
    "unpin": "",
}

# 装不下的大前缀拆成几份。没有条目的是整段放一个文件。
# 份数是按下表当时的键数定的（每份约 14 个键），键多到某份超过 800 行时把它调大。
PARTS: Dict[str, int] = {
    "appearance": 2,
    "botprofile": 2,
    "chat": 2,
    "chatbackup": 2,
    "connection": 3,
    "coreupgrade": 4,
    "friends": 4,
    "message": 3,
    "onboarding": 6,
    "peers": 4,
    "plugin": 4,
    "problem": 4,
    "tab": 3,
    "transfer": 2,
    "verification": 2,
    "verification.emoji": 5,
    "share": 3,
    "contacts": 2,
    "handoff": 1,
    "matrixlogin": 2,
}

GROUP_OF: Dict[str, str] = {
    prefix: group
    for group, prefixes in {
        "chat": "chat chatbackup chatstatus timeline message room bot botprofile transfer composer panel confirmation confirmationcard",
        "onboarding": "onboarding pairing install home homestatus launcher",
        "plugins": "plugin pluginsources store registry registrymirror tool toolpolicy tab tabs files file viewer photo preview secret sidebar sessions scope takeover port mapping portmapping",
        "peers": "peers friends invite servers connection verification lanclient lanwebui qr qrscanner camera ownerprofile keepalive share contacts handoff matrixlogin",
        "settings": "settings appearance egress notification push shortcuts shortcut",
        "common": "common problem core coreupgrade design app language startup page pin unpin",
    }.items()
    for prefix in prefixes.split()
}
GROUP_OF["verification.emoji"] = "peers"
GROUP_OF["share"] = "peers"
GROUP_OF["contacts"] = "peers"
GROUP_OF["handoff"] = "peers"
GROUP_OF["matrixlogin"] = "peers"

# 各平台的默认文件（common 的基准文件）。
BASE_FILE = {"ios": "Localizable.xcstrings", "android": "strings.xml", "fluent": "common.ftl"}
SEPARATOR = {"ios": ".", "android": "_", "fluent": "-"}

_STEM_RE = re.compile(r"^(?P<stem>.*?)(?P<part>\d+)?$")


def segments_of(key: str, platform: str) -> Tuple[str, ...]:
    """键按平台的分隔符切成小写的段。"""
    return tuple(part.lower() for part in key.split(SEPARATOR[platform]))


def prefix_of(key: str, platform: str) -> str:
    """键自己的前缀：两段（SAS 表情）优先，其次一段；没有分隔符的键没有前缀。"""
    segments = segments_of(key, platform)
    for size in (2, 1):
        if len(segments) > size and ".".join(segments[:size]) in PREFIXES:
            return ".".join(segments[:size])
    return ""


def _part(key: str, segments: Tuple[str, ...], size: int, count: int) -> int:
    """拆开的文件取第几份：编号键（SAS 表情）对份数取模，和壳里动态取表的写法一致；其余按键的 crc32。"""
    tail = segments[size] if len(segments) > size else ""
    if tail.isdigit():
        return int(tail) % count
    return zlib.crc32(key.lower().encode()) % count


def resolve(key: str, platform: str) -> Tuple[str, str]:
    """键该去的（组，文件主干）。不认识的前缀进 common 的基准文件。"""
    prefix = prefix_of(key, platform)
    stem = PREFIXES.get(prefix, "")
    # Fluent 的基准文件就叫 common.ftl：common 组的主干和基准是同一个文件，不然 core-error-* 放哪都报错。
    if platform == "fluent" and stem == BASE_FILE["fluent"][: -len(".ftl")]:
        stem = ""
    count = PARTS.get(prefix, 1)
    if count == 1:
        return GROUP_OF.get(prefix, "common"), stem
    segments = segments_of(key, platform)
    size = 2 if "." in prefix else 1
    return GROUP_OF[prefix], f"{stem}{_part(key, segments, size, count)}"


def snake(stem: str) -> str:
    return "".join(f"_{char.lower()}" if char.isupper() else char for char in stem)


def file_name(stem: str, platform: str) -> str:
    """报错里给人看的文件名。"""
    if not stem:
        return BASE_FILE[platform]
    return f"strings_{snake(stem)}.xml" if platform == "android" else f"{stem if platform == 'ios' else snake(stem)}.{'xcstrings' if platform == 'ios' else 'ftl'}"


def matching_prefixes(stem: str) -> Tuple[str, ...]:
    """这个文件允许的前缀（报错用）。"""
    match = _STEM_RE.match(stem)
    if not match:
        return ()
    return tuple(
        prefix
        for prefix, own in PREFIXES.items()
        if own == stem or (match["part"] is not None and own == match["stem"])
    )


def prefix_home(prefix: str) -> str:
    """前缀的基准文件主干。"""
    return PREFIXES.get(prefix, "")
