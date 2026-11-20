"""旁聊：主对话里的 Matrix 话题收成卡片，一轮结束在主对话里留一行结果，主对话下一轮知道。

Hermes 一个 Matrix 话题（m.thread）一个会话。这个模块做三件事：

- 话题会话的一轮在 ``on_session_end`` 收尾时，往主时间线（不挂 m.thread）发一行结果：答完了
  ``done``，失败或被打断 ``failed``；话题会话被重置（``on_session_reset``，话题里 ``/new`` 要
  主人回 ``!approve`` 确认后才触发）发 ``ended``。
- 主会话（会话键里没有 thread_id 的那个）每一轮开始前（``pre_llm_call``），把上一轮之后有新
  结果的旁聊列进这一轮的上下文：标题、状态、AI 在那个旁聊里最后一次回复的第一段（≤200 字符）。
- 在每个私聊写声明状态事件 ``app.wonton.bot.side_chats``，state_key 是 Bot 自己的 Matrix ID；
  成员 ≤2 就当私聊，和适配器判 DM 的规则一致。只对 Matrix 发结果行：Telegram 的话题也会分会话，
  但结果行是 Matrix 旁聊的约定。

结局取自 ``on_session_end`` 而不是 ``post_llm_call``：上游 ``post_llm_call`` 不带这一轮的结局，
被打断时根本不触发，而 ``on_session_end`` 带 completed/failed/interrupted、每轮恰好一次；回复
摘录（``post_llm_call`` 的 assistant_response）仍在它里取，两个钩子每轮先 post 后 end。

发消息和写状态事件经适配器的私有方法（``_send_room_message``、``_client``、``_joined_rooms``）：
适配器没有公开的发送入口（云吞的旁聊约定还没给它留接口），升级 Hermes 时先复查这几处。

会话对应、标题、摘录都在内存里：Bot 重启就丢，重启前后的结果行不补发；App 对没有结果行的旁聊
显示「没收到结果」，不会卡在错误状态。发结果行失败只记日志、不重试成死循环——不能打断主人的对话。
"""

import asyncio
import logging
import os
import re

from .locale import language
from .texts import (
    SIDE_CHAT_BODY,
    SIDE_CHAT_DIGEST,
    SIDE_CHAT_DIGEST_BRIEF,
    SIDE_CHAT_HEADER,
    SIDE_CHAT_STATE,
)

logger = logging.getLogger(__name__)

DECLARATION = "app.wonton.bot.side_chats"
EXCERPT_MAX = 200

# 网关跨线程调钩子：loop 是网关的事件循环（pre_gateway_dispatch 在它上面跑），发送都调度回去；
# inflight 记已调度的协程，测试用 _drain 等它们跑完。
_S = {
    "loop": None,
    "adapter": None,
    "session_store": None,
    "sessions": {},   # 会话 id -> (房间, 话题根)
    "threads": {},    # (房间, 话题根) -> {title, root_title, excerpt, result, fresh}
    "senders": {},    # 房间 -> 最后说话的人（结果行按他的语言写）
    "declared": set(),
    "enumerated": False,
    "inflight": set(),
}


def register(ctx):
    ctx.register_hook("pre_gateway_dispatch", pre_gateway_dispatch)
    ctx.register_hook("pre_llm_call", pre_llm_call)
    ctx.register_hook("post_llm_call", post_llm_call)
    ctx.register_hook("on_session_end", on_session_end)
    ctx.register_hook("on_session_reset", on_session_reset)


def _is_matrix(platform) -> bool:
    value = getattr(platform, "value", platform)
    return str(value or "").lower() == "matrix"


def _adapter():
    if _S["adapter"] is not None:
        return _S["adapter"]
    try:
        from gateway.config import Platform
        from gateway.run import _gateway_runner_ref

        runner = _gateway_runner_ref()
        if runner is not None:
            _S["adapter"] = runner.adapters.get(Platform.MATRIX)
    except Exception as exc:
        logger.warning("拿不到 Matrix 适配器：%r", exc)
    return _S["adapter"]


def _client():
    adapter = _adapter()
    return getattr(adapter, "_client", None) if adapter else None


def _schedule(coro):
    """把发送调度到网关的事件循环上但不等待：钩子可能跑在别的线程，原地等会死锁。"""
    loop = _S["loop"]
    if loop is None or loop.is_closed():
        coro.close()
        logger.warning("旁聊的发送被丢弃：网关事件循环不可用")
        return
    future = asyncio.run_coroutine_threadsafe(coro, loop)
    _S["inflight"].add(future)
    future.add_done_callback(_S["inflight"].discard)


async def _drain():
    """等所有已调度的发送结束（测试用）。"""
    while _S["inflight"]:
        await asyncio.wait({asyncio.wrap_future(f) for f in set(_S["inflight"])})


def _origin(session_id):
    store = _S["session_store"]
    if store is None or not session_id:
        return None
    try:
        entry = store.lookup_by_session_id(session_id)
    except Exception as exc:
        logger.warning("查会话来源失败 session=%s：%r", session_id, exc)
        return None
    return getattr(entry, "origin", None) if entry else None


def _room_thread(session_id):
    """会话 id → (房间, 话题根)，只认 Matrix 的会话。先查会话库，再查注册时记下的对应关系——
    会话被重置后，库里的旧条目没了，只剩自己记的。"""
    origin = _origin(session_id)
    if origin is not None and not _is_matrix(getattr(origin, "platform", None)):
        return None, None
    room = getattr(origin, "chat_id", None)
    thread = getattr(origin, "thread_id", None) or None
    if room is not None:
        if thread and session_id:
            _S["sessions"][session_id] = (room, thread)
        return room, thread
    return _S["sessions"].get(session_id, (None, None))


def pre_gateway_dispatch(event=None, gateway=None, session_store=None, **_):
    """每条进来的消息：记住网关、会话库和事件循环；把会话对应到房间和话题根；记话题标题的
    底稿（根消息是 m.notice，适配器不处理，标题只能从进话题的第一句猜，和 App 开旁聊时的
    取法一致）；私聊里写声明。永远放行。"""
    try:
        _S["loop"] = asyncio.get_running_loop()
    except RuntimeError:
        pass
    if session_store is not None:
        _S["session_store"] = session_store
    if gateway is not None:
        try:
            from gateway.config import Platform

            _S["adapter"] = gateway.adapters.get(Platform.MATRIX)
        except Exception as exc:
            logger.warning("从网关拿 Matrix 适配器失败：%r", exc)
    source = getattr(event, "source", None)
    room = getattr(source, "chat_id", None)
    if room is not None and _is_matrix(getattr(source, "platform", None)):
        thread = getattr(source, "thread_id", None) or None
        sender = getattr(source, "user_id", None)
        if sender:
            _S["senders"][room] = sender
        if thread:
            cell = _S["threads"].setdefault((room, thread), {})
            if not cell.get("title"):
                first_line = str(getattr(event, "text", "") or "").split("\n", 1)[0].strip()
                if first_line and not first_line.startswith("/"):
                    # 斜杠命令（/new 之类）不是开旁聊的那句话，不当标题。
                    cell["title"] = first_line[:20]
        if session_store is not None:
            try:
                sid = session_store.peek_session_id(session_store._generate_session_key(source))
                if sid:
                    _S["sessions"][sid] = (room, thread)
            except Exception as exc:
                logger.debug("记会话对应关系失败：%r", exc)
        if getattr(source, "chat_type", None) == "dm":
            _declare(room)
    _declare_all_dm_rooms_once()
    return None


async def pre_llm_call(session_id="", **_):
    """主会话的这一轮带上有新结果的旁聊；话题会话返回 None 不注入。"""
    room, thread = _room_thread(session_id)
    if thread:
        return None
    lang = await language(_S["senders"].get(room))
    lines = []
    for (thread_room, thread_root), cell in list(_S["threads"].items()):
        if thread_room != room or not cell.get("fresh"):
            continue
        cell["fresh"] = False
        title = cell.get("root_title") or cell.get("title") or thread_root[:20]
        label = SIDE_CHAT_STATE[cell.get("result") or "done"][lang]
        excerpt = cell.get("excerpt") or ""
        if excerpt:
            line = SIDE_CHAT_DIGEST[lang].format(title=title, state=label, excerpt=excerpt)
        else:
            line = SIDE_CHAT_DIGEST_BRIEF[lang].format(title=title, state=label)
        lines.append(f"- {line}")
    if not lines:
        return None
    return {"context": SIDE_CHAT_HEADER[lang] + "\n" + "\n".join(lines)}


def post_llm_call(session_id="", assistant_response="", **_):
    """记这一轮的回复摘录（主会话注入用）；发不发结果行由 on_session_end 按 结局 决定。"""
    room, thread = _room_thread(session_id)
    if not thread:
        return None
    _S["threads"].setdefault((room, thread), {})["excerpt"] = _first_paragraph(assistant_response or "")
    return None


def on_session_end(session_id="", completed=False, **_):
    """一轮收尾：话题会话发结果行，completed 之外（失败、被打断、没跑完）都是 failed。"""
    room, thread = _room_thread(session_id)
    if not thread:
        return None
    state = "done" if completed else "failed"
    _record_result(room, thread, state)
    _schedule(_send_result(room, thread, state))
    return None


def on_session_reset(session_id=None, old_session_id=None, **_):
    """会话被重置：话题会话发 ended。旧会话在库里已查不到，靠注册时记下的对应关系。"""
    room, thread = _room_thread(old_session_id or session_id)
    if not thread:
        return None
    _record_result(room, thread, "ended")
    _schedule(_send_result(room, thread, "ended"))
    return None


def _record_result(room, thread, state):
    cell = _S["threads"].setdefault((room, thread), {})
    cell["result"] = state
    cell["fresh"] = True


def _first_paragraph(text):
    """回复的第一段，按空行分段；超过 200 字符截断加 …（截短照桥的动态）。"""
    paragraph = next((part.strip() for part in re.split(r"\n\s*\n", text) if part.strip()), "")
    if len(paragraph) <= EXCERPT_MAX:
        return paragraph
    return paragraph[:EXCERPT_MAX] + "…"


async def _send_result(room, thread, state):
    title = await _title(room, thread)
    lang = await language(_S["senders"].get(room))
    content = {
        "msgtype": "m.notice",
        "body": SIDE_CHAT_BODY[lang].format(title=title, state=SIDE_CHAT_STATE[state][lang]),
        "m.relates_to": {"m.in_reply_to": {"event_id": thread}},
        "app.wonton.side_chat_result": {"v": 1, "root": thread, "title": title, "state": state},
    }
    adapter = _adapter()
    if adapter is None or getattr(adapter, "_client", None) is None:
        logger.warning("结果行发不出去：Matrix 客户端不在 room=%s root=%s", room, thread)
        return
    try:
        await adapter._send_room_message(room, content)
    except Exception as exc:
        logger.warning("结果行发送失败 room=%s root=%s：%r", room, thread, exc)


async def _title(room, thread):
    """结果行和注入用的标题：先按旁聊约定从根消息取，取不到用进话题的第一句的
    底稿，再取不到用话题根 id 的前 20 个字符（总比发不出标题强）。"""
    cell = _S["threads"].setdefault((room, thread), {})
    if cell.get("root_title"):
        return cell["root_title"]
    try:
        from_root = await _root_title(room, thread)
    except Exception as exc:
        logger.warning("读话题根失败 room=%s root=%s，用底稿当标题：%r", room, thread, exc)
        from_root = None
    if from_root:
        cell["root_title"] = from_root
        return from_root
    return cell.get("title") or thread[:20]


async def _root_title(room, root):
    """话题根的标题：根带 app.wonton.side_chat.title 就用它，否则 body 第一行的前 20 个字符。"""
    client = _client()
    if client is None:
        return None
    content = await _root_content(client, room, root)
    marked = content.get("app.wonton.side_chat")
    title = marked.get("title") if isinstance(marked, dict) else None
    if title:
        return str(title)
    body = str(content.get("body") or "")
    return body.split("\n", 1)[0].strip()[:20] or None


async def _root_content(client, room, root):
    """经网关已登录的客户端取话题根的内容 dict；加密房间先解密。mautrix 的事件类型不是 str
    枚举（EventType.ROOM_ENCRYPTED == "m.room.encrypted" 不成立，v0.21.1 实测），要按 str 比。"""
    event = await client.get_event(room, root)
    if event is not None and str(event.type) == "m.room.encrypted":
        crypto = getattr(client, "crypto", None)
        if crypto is None:
            raise RuntimeError("房间加密但客户端没有解密器")
        event = await crypto.decrypt_megolm_event(event)
    if event is None:
        raise RuntimeError("话题根解不开")
    content = event.content
    return content.serialize() if hasattr(content, "serialize") else dict(content)


def _declare(room):
    if not room or room in _S["declared"]:
        return
    _S["declared"].add(room)
    _schedule(_write_declaration(room))


def _declare_all_dm_rooms_once():
    """连上后把每个私聊补一遍声明（私聊状态事件要一直在，App 才给这个 AI 开旁聊）。
    客户端不在时等下一条消息再试。"""
    if _S["enumerated"] or _client() is None:
        return
    _S["enumerated"] = True
    _schedule(_declare_all_dm_rooms())


async def _declare_all_dm_rooms():
    adapter = _adapter()
    client = _client()
    if adapter is None or client is None:
        return
    for room in sorted(getattr(adapter, "_joined_rooms", ()) or ()):
        if room in _S["declared"]:
            continue
        try:
            members = await client.joined_members(room)
            count = len(members.members) if getattr(members, "members", None) is not None else None
        except Exception as exc:
            logger.warning("查房间成员数失败 room=%s：%r", room, exc)
            continue
        if count is not None and count <= 2:  # 适配器同样按 ≤2 判私聊
            _S["declared"].add(room)
            await _write_declaration(room)


async def _write_declaration(room):
    client = _client()
    if client is None:
        logger.warning("旁聊声明写不出去：Matrix 客户端不在 room=%s", room)
        return
    bot = os.environ.get("MATRIX_USER_ID", "").strip() or str(getattr(client, "mxid", "") or "")
    if not bot:
        logger.warning("旁聊声明写不出去：拿不到 Bot 自己的 Matrix ID room=%s", room)
        return
    try:
        # state_key 必须是 Bot 自己：App 只认 state_key 和发送者相同、发送者是房间成员的声明。
        await client.send_state_event(room, DECLARATION, {"v": 1}, state_key=bot)
    except Exception as exc:
        logger.warning("写旁聊声明失败 room=%s：%r", room, exc)
