"""把 App 一次发的「几张照片+一句话」收齐成一轮，不再一张照片各开一轮。

App 的附件区一次发送按顺序发几条事件（照片每张一条，文字挂在最后一条），每条事件内容顶层带
同一个分组标记 ``app.wonton.batch``（``v``、``id``、``index``、``count``）。Hermes 每条事件各开
一轮，这里在 ``pre_gateway_dispatch``（每条消息进网关的入口）把同组收住，到齐再交给适配器当
一轮：照片和文件按 ``index`` 排进这一轮的附件，文字用最后一条的。忙时的插话和排队由网关自己
处理——整组是一个事件，一个整体，不拆。

校验不过的标记当没有标记，照旧一条一轮：``v`` 不是 1、``id`` 不是 32 个小写十六进制字符、
``index``/``count`` 不在 0 ≤ index < count ≤ 32 里，都算。

收不齐：从这组最后到的一条算 120 秒还没到齐，就用已到的开一轮，文字末尾写明这一组几条只收到
几条，房间里再发一条 m.notice 告诉主人；之后的同 ``id`` 事件按单独一条放行，开过轮的组记一小时。

重启恢复不做：适配器对启动前的消息只留 5 秒宽限（上游 _STARTUP_GRACE_SECONDS），到钩子这里
早就过了；回读时间线要自建分页和解密，这一版重启会丢收住还没开轮的组（README 有说明）。
"""

import asyncio
import copy
import logging
import re
import time

from .locale import language
from .texts import BATCH_NOTICE, BATCH_PARTIAL

logger = logging.getLogger(__name__)

BATCH_KEY = "app.wonton.batch"
MAX_COUNT = 32
TIMEOUT_SECONDS = 120.0
SEEN_TTL_SECONDS = 3600.0
_ID_RE = re.compile(r"[0-9a-f]{32}")

# 返回 skip 网关就不派发这条消息；事件对象留在组里，到齐或超时后由我们交回去。
_SKIP = {"action": "skip", "reason": "holding wonton message batch"}

# 网关跨线程调钩子：loop 是网关的事件循环（pre_gateway_dispatch 在它上面跑），派发都调度回去；
# inflight 记已调度的协程（含组超时定时），测试用 _drain 等它们跑完。
_S = {
    "loop": None,
    "adapter": None,
    "groups": {},    # (房间, 发送者, id) -> {"count", "events", "timer"}
    "seen": {},      # id -> 过期时刻（monotonic）
    "inflight": set(),
}


def register(ctx):
    ctx.register_hook("pre_gateway_dispatch", pre_gateway_dispatch)


def pre_gateway_dispatch(event=None, gateway=None, **_):
    """每条进来的消息：合法标记的收住（skip），其余放行。这里出错就放行，不吞消息。"""
    try:
        _remember_runtime(gateway)
        return _collect(event)
    except Exception:
        logger.warning("收组判断失败，按单条放行：%r", exc_info=True)
        return None


def _remember_runtime(gateway):
    try:
        _S["loop"] = asyncio.get_running_loop()
    except RuntimeError:
        pass
    if gateway is not None:
        try:
            from gateway.config import Platform

            _S["adapter"] = gateway.adapters.get(Platform.MATRIX)
        except Exception as exc:
            logger.warning("从网关拿 Matrix 适配器失败：%r", exc)


def _collect(event):
    if event is None or _S["adapter"] is None or _S["loop"] is None:
        return None
    marker = _marker(getattr(event, "raw_message", None))
    if marker is None or _seen(marker[0]):
        return None
    source = getattr(event, "source", None)
    room = getattr(source, "chat_id", None)
    sender = getattr(source, "user_id", None)
    if not room or not sender:
        return None
    gid, index, count = marker
    key = (room, sender, gid)
    group = _S["groups"].get(key)
    if group is None:
        group = _S["groups"][key] = {"count": count, "events": {}, "timer": None}
    if index in group["events"]:  # 同 index 只收第一条：重复投递按事件去重
        return _SKIP
    group["events"][index] = event
    if len(group["events"]) >= group["count"]:
        _finish(key, group, complete=True)
    else:
        _arm(key, group)
    return _SKIP


def _marker(raw):
    """事件内容顶层的分组标记；不合规格的当没有标记。"""
    if not isinstance(raw, dict):
        return None
    field = raw.get(BATCH_KEY)
    if not isinstance(field, dict) or field.get("v") != 1:
        return None
    gid = field.get("id")
    if not isinstance(gid, str) or not _ID_RE.fullmatch(gid):
        return None
    index, count = field.get("index"), field.get("count")
    for value in (index, count):
        if not isinstance(value, int) or isinstance(value, bool):
            return None
    if not 1 <= count <= MAX_COUNT or not 0 <= index < count:
        return None
    return gid, index, count


def _seen(gid):
    deadline = _S["seen"].get(gid)
    if deadline is None:
        return False
    if deadline < time.monotonic():
        del _S["seen"][gid]
        return False
    return True


def _arm(key, group):
    """组没到齐：重新计时，120 秒从最后到的这条算起。"""
    _cancel_timer(group)
    group["timer"] = _spawn(_expire(key))


def _cancel_timer(group):
    timer = group.get("timer")
    if timer is not None:
        group["timer"] = None
        timer.cancel()


async def _expire(key):
    try:
        await asyncio.sleep(TIMEOUT_SECONDS)
    except asyncio.CancelledError:
        return
    group = _S["groups"].get(key)
    if group is not None:
        logger.info("消息组等不齐，按已到的开轮 room=%s id=%s", key[0], key[2])
        _finish(key, group, complete=False)


def _finish(key, group, complete):
    """这一组开一轮：登记 id（一小时内同 id 再到按单条放行），把事件交给适配器。"""
    _S["groups"].pop(key, None)
    _cancel_timer(group)
    now = time.monotonic()
    _S["seen"][key[2]] = now + SEEN_TTL_SECONDS
    for gid in [g for g, deadline in _S["seen"].items() if deadline < now]:
        del _S["seen"][gid]
    events = [group["events"][i] for i in sorted(group["events"])]
    _spawn(_dispatch(key, events, complete, group["count"]))


async def _dispatch(key, events, complete, total):
    """把一组并成一条事件交给适配器：最后一条当壳，附件按 index 排在前面。"""
    room, sender, gid = key
    try:
        adapter = _S["adapter"]
        if adapter is None:
            logger.warning("整组派发失败：Matrix 适配器不在 room=%s id=%s", room, gid)
            return
        combined = copy.copy(events[-1])
        urls = [url for event in events for url in (getattr(event, "media_urls", None) or [])]
        types = [kind for event in events for kind in (getattr(event, "media_types", None) or [])]
        if urls:
            combined.media_urls = urls
            combined.media_types = types
        raw = getattr(events[-1], "raw_message", None)
        if isinstance(raw, dict):
            stripped = dict(raw)
            stripped.pop(BATCH_KEY, None)
            combined.raw_message = stripped
        if not complete:
            await _note_partial(adapter, combined, room, sender, total, len(events))
        # 只交一条：忙时的 steer/排队由网关按一条消息处理，整组不拆。
        await adapter.handle_message(combined)
    except Exception:
        logger.warning("整组派发失败 room=%s id=%s：%r", room, gid, exc_info=True)


async def _note_partial(adapter, combined, room, sender, total, received):
    """没到齐就开的一轮：这一轮的文字写明缺了几条，房间里再发一条 m.notice 告诉主人。"""
    lang = await language(sender)
    partial = BATCH_PARTIAL[lang].format(total=total, received=received)
    text = combined.text or ""
    combined.text = f"{text}\n\n{partial}" if text else partial
    try:
        body = BATCH_NOTICE[lang].format(total=total, received=received)
        await adapter._send_room_message(room, {"msgtype": "m.notice", "body": body})
    except Exception as exc:
        logger.warning("缺条提示发送失败 room=%s：%r", room, exc)


def _spawn(coro):
    """把协程调度到网关的事件循环上但不等待：钩子可能跑在别的线程，原地等会死锁。"""
    loop = _S["loop"]
    if loop is None or loop.is_closed():
        coro.close()
        logger.warning("收组的动作被丢弃：网关事件循环不可用")
        return None
    future = asyncio.run_coroutine_threadsafe(coro, loop)
    _S["inflight"].add(future)
    future.add_done_callback(_S["inflight"].discard)
    return future


async def _drain():
    """等所有已调度的派发和定时结束（测试用）。"""
    while _S["inflight"]:
        await asyncio.wait({asyncio.wrap_future(f) for f in set(_S["inflight"])})
