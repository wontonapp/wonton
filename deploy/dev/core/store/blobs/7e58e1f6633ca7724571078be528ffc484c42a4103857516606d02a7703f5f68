"""每轮的身份头和调用链、停链和出箱。

云吞核心靠请求头知道 MCP 工具调用替谁说话、属于哪一轮：``X-Wonton-On-Behalf-Of`` 是说话人，
``wonton-session`` 是这一轮的会话，``app.wonton/origin`` 的 ``_meta`` 给房间和话题。Hermes 的 MCP
客户端每轮不带这些，所以在工具处理器的调用上加；值来自会话环境（见下）。

开轮时向核心报链（``POST <核心>/agent/chains/<令牌>``，带 Bot 的 Matrix 令牌），核心按说话人验证，
被拒时这一轮就以那条原因结束、不再推理；结束报 ``End``。核心停链时往出箱
（``GET/POST <核心>/agent/outbox/<令牌>``）放 ``CancelTurn``，这里实时取、打断对应会话并回报；
批准过的私聊消息（``Message``）由这里用 Bot 的 Matrix 客户端发出，出箱项 ID 当 Matrix 事务 ID，
核心重发同一条不会发两遍。

身份从 ``gateway.session_context`` 的会话环境取（``HERMES_SESSION_ID`` 等），不靠钩子里的
contextvar：``pre_llm_call`` 在单独的线程里跑，它 set 的 contextvar 回到 Agent 线程就没了。工具处理器
在 Agent 线程上跑，那里的会话环境是对的，``_run_on_mcp_loop`` 提交时上游用
``asyncio.run_coroutine_threadsafe`` 复制调用方的 contextvars，所以处理器里 set 的值在 MCP 循环上也看得到。

上游没有插件能挂的“每次工具调用”钩子，只能包它内部的处理器调用；升级 Hermes 时先复查
``tools.mcp_tool_handlers._call_tool_racing_stdio_death`` 和 ``mcp.client.session.ClientSession.call_tool``。
"""

import asyncio
import concurrent.futures
import contextvars
import logging
import os
import threading
from urllib.parse import urlsplit

import httpx

from .locale import language
from .texts import CHAIN_BUSY, CHAIN_DENIED, CHAIN_LIMIT, CHAIN_LOOP, CHAIN_SELF, CHAIN_STOPPED

logger = logging.getLogger(__name__)

ON_BEHALF_OF = "X-Wonton-On-Behalf-Of"
SESSION = "wonton-session"
BOT_TOKEN = "X-Wonton-Bot-Token"
ORIGIN = "app.wonton/origin"

# 报链开始的超时：pre_llm_call 钩子要在这里面拿到准信，才决定这一轮做不做。
START_TIMEOUT = 5.0
# 工具处理器在 MCP 循环上跑，身份在处理器外面 set，包着这次调用。
_CURRENT = contextvars.ContextVar("wonton_turn", default=None)
# 报链结束走后台单线程：钩子在网关事件循环里，同步请求会把所有会话卡住。
_CHAIN_POSTS = concurrent.futures.ThreadPoolExecutor(max_workers=1, thread_name_prefix="wonton-chain")

_S = {
    "loop": None,
    "runner": None,
    "session_store": None,
    "origins": {},   # 会话 id -> {sender, session_key, room, thread, event_id}
    "pending": {},   # 会话键 -> 同上，会话 id 这时还没有
    "active": {},    # 会话 id -> 准了的这一轮的 {sender, session, room, thread}
    "reported": set(),  # 这一轮已经报过链开始的会话 id
    "ending": set(),    # 结束时要报 End 的会话 id（准了和被拒的都要报）
    "worker": None,
    "warned": False,
}


def register(ctx):
    ctx.register_hook("pre_gateway_dispatch", pre_gateway_dispatch)
    ctx.register_hook("pre_llm_call", pre_llm_call)
    ctx.register_hook("on_session_end", on_session_end)


def _core():
    """核心地址和这个插件的总线令牌：MCP 地址是 ``<核心>/mcp/<令牌>``。拿不到时 None。"""
    url = os.environ.get("WONTON_MCP_URL", "")
    head, sep, token = url.rpartition("/mcp/")
    if not sep or not head or not token:
        return None, None
    return head.rstrip("/"), token


def _bot_token():
    return os.environ.get("MATRIX_ACCESS_TOKEN", "")


def _is_matrix(platform):
    return str(getattr(platform, "value", platform) or "").lower() == "matrix"


def _session_var(name):
    """会话环境里的一项：网关按这一轮绑定，工具处理器所在的 Agent 线程看得到。"""
    try:
        from gateway.session_context import get_session_env
    except Exception:
        return os.environ.get(name, "")
    return get_session_env(name)


def _session_id():
    return _session_var("HERMES_SESSION_ID")


def _runner():
    if _S["runner"] is not None:
        return _S["runner"]
    try:
        from gateway.run import _gateway_runner_ref

        _S["runner"] = _gateway_runner_ref()
    except Exception as exc:
        logger.warning("拿不到网关：%r", exc)
    return _S["runner"]


def pre_gateway_dispatch(event=None, gateway=None, session_store=None, **_):
    """记下这一轮的来源和会话键；第一条消息时起出箱轮询。"""
    try:
        _S["loop"] = asyncio.get_running_loop()
    except RuntimeError:
        pass
    _install_headers()
    if session_store is not None:
        _S["session_store"] = session_store
    source = getattr(event, "source", None)
    room = getattr(source, "chat_id", None)
    if room is None or not _is_matrix(getattr(source, "platform", None)):
        return None
    sender = getattr(source, "user_id", None)
    if not sender:
        return None
    session_key = None
    sid = None
    if session_store is not None:
        try:
            session_key = session_store._generate_session_key(source)
            sid = session_store.peek_session_id(session_key)
        except Exception as exc:
            logger.debug("算会话键失败：%r", exc)
    found = {
        "sender": sender,
        "session_key": session_key,
        "room": room,
        "thread": getattr(source, "thread_id", None) or None,
        "event_id": getattr(event, "message_id", None) or getattr(event, "id", None),
        "fresh": True,
    }
    if sid:
        _S["origins"][sid] = found
        _S["reported"].discard(sid)
    if session_key:
        _S["pending"][session_key] = found
    _start_worker()
    return None


def _origin(session_id):
    found = _S["origins"].get(session_id)
    if found is not None:
        return found
    store = _S["session_store"]
    if store is None or not session_id:
        return None
    try:
        entry = store.lookup_by_session_id(session_id)
    except Exception as exc:
        logger.debug("查会话来源失败 %s：%r", session_id, exc)
        return None
    origin = getattr(entry, "origin", None) if entry else None
    if origin is None or not _is_matrix(getattr(origin, "platform", None)):
        return None
    try:
        session_key = store._generate_session_key(origin)
    except Exception:
        session_key = None
    found = _S["pending"].pop(session_key, None) or {}
    found.update({
        "sender": getattr(origin, "user_id", None),
        "session_key": session_key,
        "room": getattr(origin, "chat_id", None),
        "thread": getattr(origin, "thread_id", None) or None,
    })
    found.setdefault("event_id", None)
    _S["origins"][session_id] = found
    return found


def _identity(session_id):
    """这一轮替谁说话：报链准了用准了的那份，没报上时退回记下的来源。"""
    active = _S["active"].get(session_id)
    if active is not None:
        return active
    origin = _S["origins"].get(session_id)
    if origin is None or not origin.get("sender"):
        return None
    return {
        "sender": origin["sender"],
        "session": session_id,
        "room": origin.get("room"),
        "thread": origin.get("thread"),
    }


def pre_llm_call(session_id="", sender_id="", **_):
    """报链开始；被拒时用这条原因结束这一轮。一条消息只报一次。"""
    origin = _origin(session_id) or {}
    sender = sender_id or origin.get("sender")
    if not sender or not origin.get("room") or session_id in _S["reported"]:
        return None
    body = {
        "phase": "start",
        "session": session_id,
        "room": origin.get("room") or "",
        "sender": sender,
        "event_id": origin.get("event_id") or "",
    }
    if origin.get("thread"):
        body["thread"] = origin["thread"]
    verdict, refusal, detail = _chain_start(body)
    if verdict == "retry":
        return None
    _S["reported"].add(session_id)
    origin["fresh"] = False
    if verdict == "grant":
        _S["active"][session_id] = {
            "sender": sender,
            "session": session_id,
            "room": origin.get("room"),
            "thread": origin.get("thread"),
        }
        _S["ending"].add(session_id)
        return None
    if verdict == "stopped":
        return {"action": "block", "message": CHAIN_STOPPED[_language(sender)]}
    # 其他拒绝：链没开成，也要报 End，让核心把这轮放掉。
    _S["ending"].add(session_id)
    logger.info("核心不收这一轮（%r）：%s", refusal, detail)
    return {"action": "block", "message": _refusal_text(refusal, sender)}


def _language(sender):
    """说话人的语言；问不到时英文。钩子跑在单独的线程里，那里没有事件循环。"""
    coro = language(sender)
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        try:
            return asyncio.run(coro)
        except Exception as exc:
            logger.debug("取语言失败，按英文回：%r", exc)
            return "en"
    # 调用线程里已经有事件循环（钩子改成在循环里跑也照常）：借一次性线程跑完再等。
    box = {}

    def run():
        try:
            box["lang"] = asyncio.run(coro)
        except Exception as exc:
            box["error"] = exc

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    thread.join()
    if "error" in box:
        logger.debug("取语言失败，按英文回：%r", box["error"])
        return "en"
    return box["lang"]


def _refusal_text(refusal, sender):
    """核心的拒绝类别换成给说话人看的一句话。"""
    lang = _language(sender)
    kind = refusal.get("type")
    if kind == "permission_denied":
        return CHAIN_DENIED[lang]
    if kind == "busy":
        return CHAIN_BUSY[lang]
    if kind == "invalid_input":
        return CHAIN_SELF[lang] if refusal.get("refusal") == "self_call" else CHAIN_LOOP[lang]
    return CHAIN_LIMIT[lang]


def on_session_end(session_id="", **_):
    """这一轮结束：清身份，报过的链报结束。"""
    _S["active"].pop(session_id, None)
    _S["reported"].discard(session_id)
    origin = _S["origins"].pop(session_id, None) or {}
    _S["pending"].pop(origin.get("session_key"), None)
    if session_id in _S["ending"]:
        _S["ending"].discard(session_id)
        _post_chain({"phase": "end", "session": session_id})


def _chain_start(body):
    """同步报链开始，返回 ``(结论, 拒绝体, 核心的说明)``。

    结论是 ``grant``（准了）、``stopped``、``refused``，或者 ``retry``（没拿到准信，下一次模型调用再报）。
    """
    core, token = _core()
    bot = _bot_token()
    if not core or not token or not bot:
        return "retry", {}, ""
    try:
        response = httpx.post(f"{core}/agent/chains/{token}", json=body,
                              headers={BOT_TOKEN: bot}, timeout=START_TIMEOUT, trust_env=False)
    except (httpx.HTTPError, ValueError) as exc:
        logger.warning("报链开始失败，下一次模型调用再报：%r", exc)
        return "retry", {}, ""
    detail = response.headers.get("X-Wonton-Detail", "")
    if response.is_success:
        return "grant", {}, detail
    try:
        refusal = response.json()
    except ValueError:
        refusal = None
    if not isinstance(refusal, dict) or not refusal.get("type"):
        logger.warning("报链开始被拒（%s），原因没认出来：%s", response.status_code, detail)
        return "retry", {}, detail
    return ("stopped" if refusal["type"] == "stopped" else "refused"), refusal, detail


def _post_chain(body):
    """排队 POST 一轮的结束；失败只记日志。"""
    core, token = _core()
    bot = _bot_token()
    if not core or not token or not bot:
        return
    _CHAIN_POSTS.submit(_chain_request, f"{core}/agent/chains/{token}", body, bot)


def _chain_request(url, body, bot):
    try:
        response = httpx.post(url, json=body, headers={BOT_TOKEN: bot}, timeout=START_TIMEOUT, trust_env=False)
        response.raise_for_status()
    except (httpx.HTTPError, ValueError) as exc:
        logger.warning("报调用链 %s 失败：%r", body.get("phase"), exc)


def _install_headers():
    """给 MCP 会话的每次 ``tools/call`` 带上这一轮的身份头和来源。

    插件注册时 Hermes 还没导 ``tools.mcp_tool_handlers``，第一条消息到时再包；包过以后这里是空的。
    """
    try:
        from tools import mcp_tool_handlers
    except Exception as exc:
        logger.debug("包 MCP 工具处理器失败：%r", exc)
        return
    try:
        original = mcp_tool_handlers._call_tool_racing_stdio_death
    except AttributeError as exc:
        if not _S["warned"]:
            _S["warned"] = True
            logger.warning("上游没有 MCP 工具处理器 hook 点，身份头不会带上：%r", exc)
        return
    if getattr(original, "_wonton_turn", False):
        return

    async def call(server, *args, **kwargs):
        turn = _identity(_session_id())
        if turn is None:
            return await original(server, *args, **kwargs)
        _patch_session(server.session)
        token = _CURRENT.set(turn)
        try:
            return await original(server, *args, **kwargs)
        finally:
            _CURRENT.reset(token)

    call._wonton_turn = True
    mcp_tool_handlers._call_tool_racing_stdio_death = call


def _patch_session(session):
    if getattr(session, "_wonton_turn", False):
        return
    session._wonton_turn = True
    original_headers = session._resolve_param_headers

    def resolve(name, arguments):
        headers = dict(original_headers(name, arguments) or {})
        turn = _CURRENT.get()
        if turn:
            headers[ON_BEHALF_OF] = turn["sender"]
            headers[SESSION] = turn["session"]
        return headers

    session._resolve_param_headers = resolve
    original_call = session.call_tool

    async def call_tool(name, arguments=None, *args, **kwargs):
        turn = _CURRENT.get()
        if turn and "meta" not in kwargs:
            origin = {name: turn[key] for key, name in (("room", "room_id"), ("thread", "thread_id")) if turn.get(key)}
            kwargs["meta"] = {ORIGIN: origin}
        return await original_call(name, arguments, *args, **kwargs)

    session.call_tool = call_tool


def _start_worker():
    """起后台的出箱轮询；任务留在 ``_S`` 里，事件循环只对任务持弱引用。"""
    loop = _S["loop"]
    worker = _S["worker"]
    if worker and (not hasattr(worker, "done") or not worker.done()):
        return
    if loop is None or loop.is_closed():
        return
    _S["worker"] = loop.create_task(_outbox_loop())


async def _outbox_loop():
    delay = 1
    async with httpx.AsyncClient(timeout=40, trust_env=False) as client:
        while True:
            try:
                item = await _poll_outbox(client)
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                logger.warning("取出箱失败：%r", exc)
                await asyncio.sleep(delay)
                delay = min(delay * 2, 30)
                continue
            delay = 1
            if item is None:
                continue
            try:
                await _do_outbox(item, client)
            except Exception as exc:
                logger.warning("执行出箱项失败 %r：%r", item.get("id"), exc)
                try:
                    await _receipt(item.get("id"), {"result": "failed", "category": "other"}, client)
                except Exception as receipt_exc:
                    logger.warning("回报出箱失败 %r：%r", item.get("id"), receipt_exc)


async def _poll_outbox(client=None):
    core, token = _core()
    bot = _bot_token()
    if not core or not token or not bot:
        await asyncio.sleep(5)
        return None
    if client is None:
        async with httpx.AsyncClient(timeout=40, trust_env=False) as own:
            return await _poll_outbox(own)
    response = await client.get(f"{core}/agent/outbox/{token}", headers={BOT_TOKEN: bot})
    response.raise_for_status()
    return response.json()


async def _receipt(item_id, result, client=None):
    core, token = _core()
    bot = _bot_token()
    if not core or not token or not bot or not item_id:
        return
    if client is None:
        async with httpx.AsyncClient(timeout=5, trust_env=False) as own:
            return await _receipt(item_id, result, own)
    response = await client.post(f"{core}/agent/outbox/{token}", timeout=5,
                                 json={"id": item_id, "result": result}, headers={BOT_TOKEN: bot})
    response.raise_for_status()


async def _do_outbox(item, client=None):
    item_id = item.get("id")
    kind = item.get("kind") or {}
    if kind.get("type") == "cancel_turn":
        session = kind.get("session") or ""
        origin = _origin(session) or {}
        session_key = origin.get("session_key")
        runner = _runner()
        if session_key and runner is not None:
            runner._interrupt_running_turn(
                session_key,
                interrupt_reason="云吞停链",
                invalidation_reason="wonton_chain_stopped",
            )
            await _receipt(item_id, {"result": "sent", "event_id": ""}, client)
        else:
            await _receipt(item_id, {"result": "failed", "category": "other"}, client)
    elif kind.get("type") == "message":
        event_id, category = await _send(
            kind.get("recipient") or "", kind.get("body") or "", item_id
        )
        if event_id is None:
            await _receipt(item_id, {"result": "failed", "category": category}, client)
        else:
            await _receipt(item_id, {"result": "sent", "event_id": event_id}, client)
    else:
        await _receipt(item_id, {"result": "failed", "category": "other"}, client)


def _server():
    """收件人的服务器：优先 ``MATRIX_HOMESERVER`` 的主机名，取不到再认 Bot 的 MXID。"""
    homeserver = os.environ.get("MATRIX_HOMESERVER", "")
    if homeserver:
        host = urlsplit(homeserver).hostname
        if host:
            return host
    return os.environ.get("MATRIX_USER_ID", "").partition(":")[2]


async def _send(recipient, body, txn_id):
    """以 Bot 的身份发进和收件人的私聊。

    返回 ``(event_id, category)``；没发成时 event_id 为 None，category 是 ``no_room``（没有已加入的私聊）
    或 ``other``（读账号数据、发送本身失败）。
    """
    client = _client()
    server = _server()
    if client is None or not server or not recipient:
        return None, "other"
    target = f"@{recipient}:{server}"
    try:
        direct = await client.get_account_data("m.direct") or {}
        joined = set(await client.get_joined_rooms() or [])
    except Exception as exc:
        logger.warning("读私聊房间失败：%r", exc)
        return None, "other"
    for room in direct.get(target, []):
        if room in joined:
            from mautrix.types import EventType

            try:
                event_id = await asyncio.wait_for(client.send_message_event(
                    room, EventType.ROOM_MESSAGE,
                    {"msgtype": "m.text", "body": body}, txn_id=txn_id), timeout=45)
            except Exception as exc:
                logger.warning("发私聊失败：%r", exc)
                return None, "other"
            return event_id, ""
    return None, "no_room"


def _client():
    runner = _runner()
    if runner is None:
        return None
    try:
        from gateway.config import Platform

        adapter = runner.adapters.get(Platform.MATRIX)
    except Exception as exc:
        logger.warning("拿不到 Matrix 适配器：%r", exc)
        return None
    return getattr(adapter, "_client", None)
