"""把 Hermes 自己的斜杠命令写进房间状态事件 ``app.wonton.bot.commands``，App 的“/”菜单靠它列命令。

表整张来自上游：网关能用的内置命令、Hermes 插件注册的命令、技能命令；只在 CLI 里能用的不算。
说明按主人的语言取上游的语言包，和插件的其他提示同一种取法。

网关没有“启动完成”和“加入房间”的插件钩子：第一条 Matrix 消息到达时起一个后台任务，先把已加入的房间都写一遍，
之后每条消息补写它所在的房间，任务每分钟再看一遍已加入的房间。表没变就不重写：进程内按房间记内容摘要，
动手前先读一次已有的状态，重启以后同样的表也不重写。

写用网关自己的 Matrix 客户端（Bot 的身份），不改上游源码。不写房间列表以外的房间；没权限写就在那个房间说一次，
其他失败只记日志，下一轮再试——菜单是附属信息，不挡消息。
"""

import asyncio
import hashlib
import json
import logging
import time

from mautrix.errors import MForbidden, MNotFound
from mautrix.types import EventType

from . import locale

logger = logging.getLogger(__name__)

EVENT_TYPE = "app.wonton.bot.commands"
STATE = EventType.find(EVENT_TYPE, EventType.Class.STATE)
MESSAGE = EventType.ROOM_MESSAGE

# 和 wonton-protocol 的 BotCommands 同一组上限；超出的按顺序丢，数目记进 omitted。
MAX_COMMANDS = 200
MAX_NAME_CHARS = 64
MAX_DESCRIPTION_CHARS = 256
MAX_HINT_CHARS = 128
MAX_BYTES = 60 * 1024

# 命令表缓存的秒数：技能目录和配置门是文件与配置，别每条消息都扫一遍。
TABLE_TTL = 60.0
# 已加入房间的巡检间隔。
SWEEP_SECONDS = 60.0

# 写状态、读状态、发提示各自的上限：homeserver 在插件网络里，卡住也不能占着后台任务。
READ_TIMEOUT = 15.0
WRITE_TIMEOUT = 30.0

REFUSED_BODY = {
    "en": "This room doesn't let me post the command menu. Allow members to send state events "
    "(or make me a moderator) and it will show up.",
    "zh-Hans": "这个房间不让我写命令菜单：允许成员发状态事件（或把我设成管理员）就会出来。",
    "zh-Hant": "這個房間不讓我寫指令選單：允許成員送狀態事件（或把我設成管理員）就會出現。",
    "ja": "このルームではコマンドメニューを書き込めません。メンバーの状態イベント送信を許可する"
    "（またはモデレーターにする）と表示されます。",
}

_S = {
    "runner": None,
    "loop": None,
    "worker": None,
    "inflight": set(),
    "written": {},
    "refused": set(),
    "table": None,
    "warned": False,
}


def register(ctx):
    ctx.register_hook("pre_gateway_dispatch", pre_gateway_dispatch)


def pre_gateway_dispatch(event=None, **_):
    """每条进来的消息：Matrix 的先确保后台任务在，再顺手对齐它所在的房间。永远放行。"""
    source = getattr(event, "source", None)
    if source is None or not _is_matrix(getattr(source, "platform", None)):
        return None
    try:
        _S["loop"] = asyncio.get_running_loop()
    except RuntimeError:
        return None
    _start_worker()
    room = getattr(source, "chat_id", None)
    if room:
        _spawn(_sync(room))
    return None


def _is_matrix(platform) -> bool:
    value = getattr(platform, "value", platform)
    return str(value or "").lower() == "matrix"


def _adapter():
    """适配器每次现查：掉线重连会换对象，只缓存网关。"""
    if _S["runner"] is None:
        try:
            from gateway.run import _gateway_runner_ref

            _S["runner"] = _gateway_runner_ref()
        except Exception as exc:
            logger.warning("拿不到网关：%r", exc)
    runner = _S["runner"]
    if runner is None:
        return None
    try:
        from gateway.config import Platform

        return runner.adapters.get(Platform.MATRIX)
    except Exception:
        return None


def _client():
    adapter = _adapter()
    return getattr(adapter, "_client", None) if adapter else None


def _start_worker():
    worker = _S["worker"]
    if worker is not None and not worker.done():
        return
    loop = _S["loop"]
    if loop is None or loop.is_closed():
        return
    _S["worker"] = loop.create_task(_sweep())


def _spawn(coro):
    loop = _S["loop"]
    if loop is None or loop.is_closed():
        coro.close()
        return
    task = loop.create_task(coro)
    _S["inflight"].add(task)
    task.add_done_callback(_S["inflight"].discard)


async def _sweep():
    while True:
        try:
            await _sync_all()
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            logger.warning("巡检已加入的房间失败：%r", exc)
        await asyncio.sleep(SWEEP_SECONDS)


async def _sync_all():
    client = _client()
    if client is None:
        _warn_no_client()
        return
    rooms = await asyncio.wait_for(client.get_joined_rooms(), READ_TIMEOUT)
    for room in rooms:
        await _sync(room)


async def _sync(room):
    """给一个房间对齐命令表；表没变就不写。自身的失败不往外抛。"""
    client = _client()
    if client is None:
        _warn_no_client()
        return
    bot = _bot(client)
    if not bot:
        _warn_once("MATRIX_USER_ID 为空，写不了命令表")
        return
    language = "en"
    try:
        language = await locale.language(None)
        table = _table(language)
        if table is None:
            return
        content, digest = table
        if _S["written"].get(room) == digest:
            return
        if await _current_digest(client, room, bot) == digest:
            _S["written"][room] = digest
            return
        await asyncio.wait_for(client.send_state_event(room, STATE, content, state_key=bot), WRITE_TIMEOUT)
        _S["written"][room] = digest
        _S["refused"].discard(room)
    except MForbidden:
        _S["written"].pop(room, None)
        logger.warning("房间 %s 不让 Bot 写状态事件，命令菜单放不上去", room)
        if room not in _S["refused"]:
            _S["refused"].add(room)
            await _notice(client, room, language)
    except MNotFound:
        logger.warning("房间 %s 找不到了（已退出？），跳过", room)
    except asyncio.TimeoutError:
        logger.warning("房间 %s 写命令表超时", room)
    except Exception as exc:
        logger.warning("房间 %s 写命令表失败：%r", room, exc)


async def _current_digest(client, room, bot):
    """房间里已有的内容摘要；没有这个状态事件时返回空串。"""
    try:
        content = await asyncio.wait_for(client.get_state_event(room, STATE, state_key=bot), READ_TIMEOUT)
    except MNotFound:
        return ""
    if not isinstance(content, dict):
        return ""
    return _digest(content)


async def _notice(client, room, language):
    body = REFUSED_BODY.get(language) or REFUSED_BODY["en"]
    try:
        await asyncio.wait_for(
            client.send_message_event(room, MESSAGE, {"msgtype": "m.notice", "body": body}), WRITE_TIMEOUT
        )
    except Exception as exc:
        logger.warning("房间 %s 的权限提示也发不出去：%r", room, exc)


def _warn_no_client():
    _warn_once("Matrix 客户端还没就绪，命令表下一轮再写")


def _warn_once(message):
    if _S["warned"]:
        return
    _S["warned"] = True
    logger.warning(message)


def _bot(client):
    return str(getattr(client, "mxid", "") or "").strip()


def _table(language):
    """(内容, 摘要)；上游两处都读不到时返回 None，让房间保持原样。"""
    cached = _S["table"]
    now = time.monotonic()
    if cached and cached[0] == language and cached[1] > now:
        return cached[2], cached[3]
    built = _build(language)
    if built is None:
        return None
    _S["table"] = (language, now + TABLE_TTL, *built)
    return built


def _build(language):
    rows = []
    failed = 0
    try:
        from hermes_cli.commands import COMMAND_REGISTRY, _is_gateway_available, _resolve_config_gates

        overrides = _resolve_config_gates()
        for command in COMMAND_REGISTRY:
            if _is_gateway_available(command, overrides):
                rows.append((command.name, _describe(command.name, command.description, language), command.args_hint))
    except Exception as exc:
        failed += 1
        logger.warning("读上游命令表失败，这次的菜单里没有内置命令：%r", exc)
    try:
        from hermes_cli.commands_platforms import _collect_gateway_skill_entries

        entries, _hidden = _collect_gateway_skill_entries(
            "matrix", None, {name for name, *_ in rows}, desc_limit=MAX_DESCRIPTION_CHARS
        )
        rows.extend((name, description, "") for name, description, _key, _raw in entries)
    except Exception as exc:
        failed += 1
        logger.warning("读插件和技能命令失败，这次的菜单里没有它们：%r", exc)
    if failed == 2:
        return None
    content, omitted = _normalize(rows)
    if omitted:
        content["omitted"] = omitted
    return content, _digest(content)


def _describe(name, fallback, language):
    try:
        from agent.i18n import t

        key = f"slash.{name}.description"
        text = t(key, lang=language)
        return fallback if text == key else text
    except Exception:
        return fallback


def _normalize(rows):
    """去掉不合规和重名的，按协议上限截断；返回 (内容, 丢掉的条数)。"""
    commands = []
    seen = set()
    for name, description, hint in rows:
        name = str(name or "").strip().strip("/").strip()
        if not name or name in seen or len(name) > MAX_NAME_CHARS or not name.isprintable():
            continue
        seen.add(name)
        item = {"name": name, "description": _clip(description, MAX_DESCRIPTION_CHARS)}
        if hint:
            item["hint"] = _clip(hint, MAX_HINT_CHARS)
        commands.append(item)
    dropped = max(0, len(commands) - MAX_COMMANDS)
    commands = commands[:MAX_COMMANDS]
    content = {"commands": commands}
    while commands and len(_dump(content)) > MAX_BYTES:
        commands.pop()
        dropped += 1
        content = {"commands": commands}
    return content, dropped


def _clip(text, limit):
    text = " ".join(str(text or "").split())
    if len(text) <= limit:
        return text
    return text[: limit - 1].rstrip() + "…"


def _dump(content):
    return json.dumps(content, ensure_ascii=False, separators=(",", ":")).encode()


def _digest(content):
    return hashlib.sha256(_dump(content)).hexdigest()
