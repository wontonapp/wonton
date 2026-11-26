"""每轮的身份头和调用链、停链和出箱。

云吞核心靠请求头知道 MCP 工具调用替谁说话、属于哪一轮：``X-Wonton-On-Behalf-Of`` 是说话人，
``wonton-session`` 是这一轮的会话，``app.wonton/origin`` 的 ``_meta`` 给房间和话题。Hermes 的 MCP
客户端每轮不带这些，所以在工具处理器的调用上加；值来自工具调用自己 context 的
``HERMES_SESSION_ID``（见下）。

开轮时向核心报链（``POST <核心>/agent/chains/<令牌>``，带 Bot 的 Matrix 令牌），核心按说话人验证，
被拒时这一轮就以那条原因结束、不再推理；结束报 ``End``。核心停链时往出箱
（``GET/POST <核心>/agent/outbox/<令牌>``）放 ``CancelTurn``，这里实时取、打断对应会话并回报；
批准过的私聊消息（``Message``）由这里用 Bot 的 Matrix 客户端发出，出箱项 ID 当 Matrix 事务 ID，
核心重发同一条不会发两遍。

身份靠工具调用自己 context 里的 ``HERMES_SESSION_ID``：上游在 Agent 线程的 context 里按这一轮绑好它，
MCP 的工具线程拷着那份 context 跑，所以拿到的是这一轮自己的会话号，同时几轮也不会串。
``pre_llm_call`` 在别的线程、拷的上下文里跑，它设的 contextvar 回不到工具线程（0.22.7 实测 3/3 次工具调用
都走会话环境那条），所以身份按会话号查 ``_S["active"]`` 和 ``_S["origins"]`` 两张表，不靠“最近一轮”的进程级值。
只有会话号时带 ``wonton-session``、不带代表头，让核心按会话去认；连会话号也取不到才不带身份。

定时任务跑的时候没有人当场说话，会话环境里也只有 Hermes 自己生成的 cron 会话号，``_identity`` 查不到；
这时按插件上下文的 ``current_cron_execution()`` 认出手上的任务，读它的 origin 当说话人（见 ``_cron_identity``）。
委派出去的子代理里上游让它返回 None，改读上游自己的执行标记，子代理同样替创建者说话。
定时任务是上游从 cron 直接进 Agent 的，没有网关消息，所以工具处理器也在 ``pre_llm_call`` 里包。

``tools/list`` 也盖这组头：核心按说话人封顶广告出去的清单，连接、重连时取的清单会挂着连接那一刻的帽子，
所以每轮模型调用前按这一轮的说话人重取一遍，再把新工具交给这一轮。

上游没有插件能挂的“每次工具调用”钩子，只能包它内部的处理器调用；升级 Hermes 时先复查
``tools.mcp_tool_handlers._call_tool_racing_stdio_death``、``mcp.client.session.ClientSession.call_tool`` /
``list_tools``、``tools.mcp_tool._servers`` 和 ``tools.mcp_tool_agent.refresh_agent_mcp_tools``。
"""

import asyncio
import concurrent.futures
import contextlib
import contextvars
import logging
import os
import sys
import threading
from urllib.parse import urlsplit

import httpx

from .locale import language
from .texts import CHAIN_DENIED, CHAIN_LIMIT, CHAIN_LOOP, CHAIN_OTHER, CHAIN_SELF, CHAIN_STOPPED

logger = logging.getLogger(__name__)

try:
    import gateway.session_context as _session_context
except Exception:  # 上游老版本没有它：只认环境变量
    _session_context = None

ON_BEHALF_OF = "X-Wonton-On-Behalf-Of"
SESSION = "wonton-session"
BOT_TOKEN = "X-Wonton-Bot-Token"
ORIGIN = "app.wonton/origin"

# 报链开始的超时：pre_llm_call 钩子要在这里面拿到准信，才决定这一轮做不做。
START_TIMEOUT = 5.0
# 重取工具清单的超时：也在 pre_llm_call 里，等它把清单交上来，模型才有工具可用。
REFRESH_TIMEOUT = 10.0
# 工具处理器在 MCP 循环上跑，身份在处理器外面 set，包着这次调用：仍然是个 contextvar，但在
# 每次调用自己的任务里 set，值跟着任务走，不看进程里最近是哪一轮。
_CURRENT = contextvars.ContextVar("wonton_turn", default=None)
# 报链结束走后台单线程：钩子在网关事件循环里，同步请求会把所有会话卡住。
_CHAIN_POSTS = concurrent.futures.ThreadPoolExecutor(max_workers=1, thread_name_prefix="wonton-chain")

_S = {
    "loop": None,
    "runner": None,
    "session_store": None,
    "gateway": None,   # 网关：pre_llm_call 的钩子线程上按会话号从它的 agent 缓存找这一轮
    "ctx": None,     # 插件上下文：定时任务执行中从它拿这条执行
    "origins": {},   # 会话 id -> {sender, session_key, room, thread, event_id}
    "pending": {},   # 会话键 -> 同上，会话 id 这时还没有
    "active": {},    # 会话 id -> 准了的这一轮的 {sender, session, room, thread}
    "reported": set(),  # 这一轮已经报过链开始的会话 id
    "ending": set(),    # 结束时要报 End 的会话 id（准了和被拒的都要报）
    "unknown": set(),   # 已经提醒过“身份取不到”的会话环境值；空串在每轮结束时重新提醒
    "cron_origins": {}, # 任务 id -> 身份或 None：一次执行里 origin 不变，读到过就记住
    "cron_warned": {},  # 任务 id -> 提醒过的那份 origin，同一份只提醒一次
    "last": None,       # 这一轮的身份：清单没有进行中的调用，给它用
    "listed": None,     # 清单当前是按谁的封顶取的
    "warned_list": False,
    "warned_marker": False,
    "worker": None,
    "webs": {},      # 会话 id -> 刚提交这条会话的网页连接（见 pages.py）
    "server": None,  # 上游会话表所在的模块，由 pages 装上（gateway 是网关本身，不是它）
    "page_warned": set(),  # 已经报过“上游会话表换了形状”的会话 id
    "dispatched": {},  # 会话 id -> {待认领的发送人: origin}：只有 sender_id 对上的那一轮认领，结束不清、重置才清
    "warned": False,
}


def register(ctx):
    _S["ctx"] = ctx
    ctx.register_hook("pre_gateway_dispatch", pre_gateway_dispatch)
    ctx.register_hook("pre_llm_call", pre_llm_call)
    ctx.register_hook("on_session_end", on_session_end)
    ctx.register_hook("on_session_reset", on_session_reset)


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
    """会话环境里的一项。

    上游把它存在 contextvar 里，网关按这一轮在 Agent 线程的 context 里绑定，MCP 的工具线程拷着那份
    context 跑，所以看到的是这一轮自己的值。这一份 context 里从没绑过时（``_UNSET``），上游回落到
    ``os.environ`` 的旧值；清掉是绑成空串，这时拿到的是空串，不再回落。
    """
    context = sys.modules.get("gateway.session_context") or _session_context
    get = getattr(context, "get_session_env", None)
    return get(name) if get is not None else os.environ.get(name, "")


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
    if gateway is not None:
        _S["gateway"] = gateway
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
        "source": source,
        "session_key": session_key,
        "room": room,
        "thread": getattr(source, "thread_id", None) or None,
        "event_id": getattr(event, "message_id", None) or getattr(event, "id", None),
    }
    if sid:
        _S["origins"][sid] = found
        _S["dispatched"].setdefault(sid, {})[sender] = found
        _S["reported"].discard(sid)
    if session_key:
        _S["pending"][session_key] = found
    _start_worker()
    return None


def _origin(session_id):
    found = _S["origins"].get(session_id)
    if found is not None:
        # 同一条会话的下一句：记在 pending 里的是这一句的消息和来源，盖掉上一轮的
        # （被打断或重置以后 peek 不到会话 id，pre_gateway_dispatch 只写 pending）。
        fresh = _S["pending"].pop(found.get("session_key"), None)
        if fresh is not None:
            found.update(fresh)
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
    """这一轮替谁说话，连同这份身份是哪来的。

    按工具调用自己的会话号查这一轮的来源；说话人取不到时不做猜测，带会话号不带代表头，让核心按会话去认
    ——猜错了卡片就挂到别人的链上。会话号也没有时返回 None，调用不带身份（核心的审计里看得到）。

    会话结束以后迟到的调用只剩会话号这一支：核心对已结束的会话没有在跑的轮，会退到“封顶最严的轮”，
    这条会话再开新链之前可能拿到别的说话人，和完全不带会话头一样，不是准的。
    """
    active = _S["active"].get(session_id)
    if active is not None:
        return active, "会话环境"
    origin = _S["origins"].get(session_id)
    if origin is not None:
        return {
            "sender": origin.get("sender"),
            "session": session_id,
            "room": origin.get("room"),
            "thread": origin.get("thread"),
        }, "来源记录"
    if session_id:
        return {"sender": None, "session": session_id, "room": None, "thread": None}, "只有会话"
    return None, "无"


def note_page():
    """网页控制台连上来了：出箱轮询和 MCP 头只有网关的入站消息会带上，网页不经过它。"""
    try:
        _S["loop"] = asyncio.get_running_loop()
    except RuntimeError:
        pass
    _install_headers()
    _start_worker()


def attach_gateway(server):
    """记下上游的会话表模块：网页连接关了要按它认人。网关本身在 ``_S["gateway"]``，两回事。"""
    _S["server"] = server


def note_peer(session_id, transport):
    """这条连接刚提交（或恢复）了这条会话。谁都记：终端页签最后提交时不该让早先的网页顶上。"""
    if session_id:
        _S["webs"][session_id] = transport
    _pages().remember(session_id, transport)


def _pages():
    from . import pages

    return pages


def _page_user(transport):
    """这条连接带的打开者；不是网页连接时 None。"""
    return _pages().opener_of(transport)


def _warn_pages(session_id, reason):
    """上游形状变了按会话各报一次：同一进程里另一个会话的形状变化还得能看见。"""
    if session_id in _S["page_warned"]:
        return None
    _S["page_warned"].add(session_id)
    logger.warning("%s，网页连接认不出人（会话 %s）", reason, session_id)
    return None


def _page_sender(session_id):
    """网页这一轮替谁说话：最后提交这条会话、带着打开者的连接。

    连接在控制台进程里、这一轮在网关进程里跑，提交者从盘上读（``pages.remember`` 写的）。盘上记着却没人
    （终端页签最后提交的）就不算人。记下的连接已经不在会话里时（关了、被上游拆掉）改由还剩的活连接里最后
    提交的那条说；没有会话表或一个活连接都没有才退回记录本身。一个带打开者的都没有就不算人，交回上游的
    ``sender_id``。网页会话没有 Matrix 来源，``pre_gateway_dispatch`` 不来；按轮现取，不缓存到会话。
    """
    recorded = _pages().recorded(session_id)
    server = _S["server"]
    live = None
    if server is None:
        # 没有控制台的进程（定时任务、旧版环境）是常态，不当事故报。
        logger.debug("没有上游的会话表，网页连接认不出人（会话 %s）", session_id)
    else:
        try:
            live = server._session_live_transports(server._sessions.get(session_id))
        except AttributeError as exc:
            _warn_pages(session_id, f"上游的会话表换了形状：{exc!r}")
    remembered = _S["webs"].get(session_id)
    # 记下的连接已经不在会话里了（上游拆 Fanout 时子连接的 ``_closed`` 不置位，光看标记发现不了）：记录
    # 就是它的话已经过时，改由还剩的活连接里最后提交的那条说。别的控制台进程里刚打完字的记录本进程
    # 不认识它的连接，照旧信记录。
    stale = (remembered is not None and live is not None
             and not any(peer is remembered for peer in live)
             and (recorded is None or _page_user(remembered) == recorded.strip()))
    if live and stale:
        for transport in reversed(live):
            sender = _page_user(transport)
            if sender:
                return sender
    if recorded is not None:
        return recorded.strip() or None
    if remembered is not None:
        return _page_user(remembered)
    return None


def _cron_execution():
    """手上的定时任务执行，没有时 None。

    先问插件上下文；委派出去的子代理里上游特意让它返回 None（``cron/execution_identity.py``），
    这时退回读上游自己那份执行标记：子代理跑在委派时拷来的 context 里，标记还在，它替同一件事干活。
    """
    ctx = _S["ctx"]
    execution = ctx.current_cron_execution() if ctx is not None else None
    if execution is not None:
        return execution
    try:
        from cron.execution_identity import _CURRENT

        return _CURRENT.get()
    except Exception as exc:
        if not _S["warned_marker"]:
            _S["warned_marker"] = True
            logger.warning("拿不到上游的定时任务执行标记，子代理那条身份会失去：%r", exc)
        return None


def _warn_cron(job_id, origin, reason):
    """同一个任务的同一份 origin 只提醒一次：主人改了任务的 origin 会再提醒一次。"""
    key = repr(origin)
    if _S["cron_warned"].get(job_id) == key:
        return
    if len(_S["cron_warned"]) > 256:
        _S["cron_warned"].clear()
    _S["cron_warned"][job_id] = key
    logger.warning("定时任务 %s 的 origin 用不上（%s），调用不带身份", job_id, reason)


def _cron_identity(execution):
    """定时任务（含它委派出去的子代理）调工具时替这条任务的创建者说话。没有定时任务时 None。

    定时任务跑的时候没有人当场说话，`_identity` 查不到；任务的 ``origin`` 记着建任务那一轮的会话环境
    （上游 ``tools/cronjob_job_args._origin_from_env``），``origin`` 是建任务时从会话环境抄的字符串，
    A 让 Bot 建的任务就按 A 算，模型在工具子进程里改不了它。

    头是插件自报的，核心只核对“这个 Bot 听不听这个人”（和聊天轮一样）：创建者后来不听这个 Bot 了，核心按
    认不出算、只读。平台不是正好 ``matrix``、没有创建者、任务没了都不带身份，那几样主人从日志里看不出，所以
    各提醒一次；没有身份核心按只读算（核心 #832 起），不按 Bot 的归属人，也不再按共享 Bot 的能用人算。

    定时任务没有进行中的轮次，子代理也不是一轮，所以只带代表头，不带 ``wonton-session``。
    """
    job_id = getattr(execution, "job_id", None)
    if not job_id:
        _warn_cron("", None, "执行里没有任务 id")
        return None
    # 一次执行里 origin 不会变；主人改了任务，下一次执行的执行号不同，会重新读。
    cache_key = _cron_key(job_id, execution)
    cache = _S["cron_origins"]
    if cache_key in cache:
        identity = cache[cache_key]
        return dict(identity) if identity else None
    try:
        from cron.jobs import get_job

        job = get_job(job_id)
    except Exception as exc:
        # 读盘失败可能是暂时的，不记住，下一次调用再试。
        _warn_cron(job_id, None, f"读任务失败 {exc!r}")
        return None
    origin = (job or {}).get("origin") or {}
    sender = origin.get("user_id")
    if job is None:
        _warn_cron(job_id, origin, "任务没了，不带身份（核心按只读算）")
        identity = None
    elif origin.get("platform") != "matrix":
        _warn_cron(job_id, origin, f"建任务的平台是 {origin.get('platform')!r}，不带身份（核心按只读算）")
        identity = None
    elif not sender:
        _warn_cron(job_id, origin, "origin 里没有创建者，不带身份（核心按只读算）")
        identity = None
    else:
        identity = {"sender": sender, "room": origin.get("chat_id"), "thread": origin.get("thread_id")}
    if len(cache) > 64:
        cache.clear()
    cache[cache_key] = identity
    return dict(identity) if identity else None


def _cron_key(job_id, execution):
    """一次执行一份缓存：换个执行号（主人改过任务）就重新读 origin。"""
    return f"{job_id}\n{getattr(execution, 'execution_id', '')}"


async def _cron_identity_async(execution):
    """[`_cron_identity`] 的异步版：缓存没命中时把 ``get_job`` 的读盘和解析挪到线程里。

    `get_job` 是同步的，会把整份 ``jobs.json`` 读出来解一遍；在事件循环里直接调用会挡住别的会话。
    """
    identity = _S["cron_origins"].get(_cron_key(getattr(execution, "job_id", None), execution), ())
    if identity != ():
        return dict(identity) if identity else None
    return await asyncio.to_thread(_cron_identity, execution)


def _refresh_tools():
    """在 MCP 循环上把每个在线服务的清单按当前身份重取一遍。

    服务端那侧的 ``tools/list`` 拿的是 ``_S["last"]``：``_run_on_mcp_loop`` 拷的是循环线程的 context，工具
    线程里 set 的 ``_CURRENT`` 到不了。只连着读，不重连（不是 ``reprobe_tool_availability``）。全都失败时抛错，
    一个都没连上时返回 False——那样不记说话人，下一轮再来，连接自己的第一次清单会带这一轮的帽子。
    """
    from tools.mcp_tool import _servers
    from tools.mcp_tool_loop import _run_on_mcp_loop

    tasks = [task for task in _servers.values() if task.session is not None]
    if not tasks:
        return False
    errors = []
    for server_task in tasks:
        try:
            _run_on_mcp_loop(lambda task=server_task: task._refresh_tools(), timeout=REFRESH_TIMEOUT)
        except Exception as exc:
            errors.append(exc)
    if len(errors) == len(tasks):
        raise errors[0]
    for error in errors:
        logger.warning("有一个 MCP 服务的清单没重取上：%r", error)
    return True


def _refresh_agent_tools(agent):
    """把注册表里现在的工具重新交给这一轮。

    ``preserve_prefix`` 只往后面加、不动模型已经见过的顺序；这样重取到的写工具这一轮就能用，不用等下一轮。
    """
    from tools.mcp_tool_agent import refresh_agent_mcp_tools

    return refresh_agent_mcp_tools(agent, quiet_mode=True, preserve_prefix=True)


def _agent_for_turn(session_id):
    """这一轮正在跑的 Agent；工具清单挂在它身上。

    ``pre_llm_call`` 没有这个参数，上面几层调用方有（``_collect_pre_llm_call_context(agent, …)``）。上游把
    ``pre_llm_call`` 扔在 ``hermes-hook-*`` 线程上跑（``plugins_dispatch`` 的 bounded 集合），调用点的栈帧到不了
    钩子线程，所以先按栈找（同线程的调用点），再按会话号去网关的 agent 缓存里找。只认会话号对得上的那一个，
    找不到就 None，新清单从下一轮起生效。
    """
    frame = sys._getframe(1)
    while frame is not None:
        agent = frame.f_locals.get("agent")
        if agent is not None and getattr(agent, "session_id", None) == session_id:
            return agent
        frame = frame.f_back
    cache = getattr(_S["gateway"], "_agent_cache", None)
    if not cache:
        return None
    lock = getattr(_S["gateway"], "_agent_cache_lock", None)
    with (lock or contextlib.suppress()):
        for entry in list(cache.values()):
            agent = entry[0] if isinstance(entry, tuple) else entry
            if getattr(agent, "session_id", None) == session_id:
                return agent
    return None


def _remember_for_listing(session_id, sender):
    """按这一轮的说话人重取工具清单，并把身份留给 ``tools/list``。

    核心按代表头过滤广告出去的清单：共享 Bot 的清单按能用人里最低的那位封顶，写工具会被整片摘掉，而这份
    清单只在连接、重连时取一次，挂着不动的连接会一直用连接那一刻的帽子。模型调用之前按这一轮的身份重取
    一遍，模型这一轮才看得见他买得起的工具。同一个说话人不重取。
    """
    execution = _cron_execution()
    if execution is not None:
        identity = _cron_identity(execution)
    elif sender:
        identity = {"sender": sender, "session": session_id, "room": None, "thread": None}
    else:
        identity = None
    _S["last"] = identity
    if identity is None or identity["sender"] == _S["listed"]:
        return
    try:
        refreshed = _refresh_tools()
    except Exception as exc:
        logger.warning("按 %s 重取工具清单失败，这一轮按旧清单走：%r", identity["sender"], exc)
        return
    if not refreshed:
        return
    if _S["last"] is not identity:
        return  # 重取期间这一轮已经换人或没了，别把旧身份记成已取
    _S["listed"] = identity["sender"]
    agent = _agent_for_turn(session_id)
    if agent is None:
        return
    try:
        added = _refresh_agent_tools(agent)
    except Exception as exc:
        logger.warning("把重取到的清单交给这一轮失败：%r", exc)
        return
    if added:
        logger.warning("按 %s 重取后清单多了：%s", identity["sender"], ", ".join(sorted(added)))


def _claim_dispatch(session_id, sender_id):
    """这一轮是不是网关派发的那条 IM；是就把它自己的来源交出来并清掉标记。

    只认说话人对上的那一条：网页开着这条会话时，先跑到的网页轮、终端页签轮（``sender_id`` 是
    ``uid:<n>``）都不能把标记用掉；同一条会话几条 IM 连着来时，各人认领各人的来源，后一条盖掉
    ``_S["origins"]`` 也不影响前一条。
    """
    if not sender_id:
        return None
    waiting = _S["dispatched"].get(session_id)
    if not waiting:
        return None
    found = waiting.pop(sender_id, None)
    if found is None:
        return None
    if not waiting:
        _S["dispatched"].pop(session_id, None)
    # 这个人自己那条消息要另报一次：派发进来的下一轮可能赶在上一条还能跑的时候到，``reported`` 会挡住它。
    _S["reported"].discard(session_id)
    return found


def pre_llm_call(session_id="", sender_id="", **_):
    """报链开始；被拒时用这条原因结束这一轮。一条消息只报一次。"""
    # 定时任务没有网关消息，pre_gateway_dispatch 不会来，只靠这里把工具处理器包上。
    _install_headers()
    # 网关派发的这一轮（IM 消息）按 IM 的人、房间和事件号算：网页开着这条会话时，开页的人也不能顶掉它。
    # 标记按会话号挂在说话人名下，对上的人才认领（控制台是 ``cloud-gateway``，App 页签是 ``uid:<n>``）。
    dispatched = _claim_dispatch(session_id, sender_id)
    if dispatched is not None:
        _S["origins"][session_id] = dispatched
        origin = dispatched
    else:
        origin = _origin(session_id) or {}
    page = None if dispatched is not None else _page_sender(session_id)
    if page is not None:
        # 网页轮不带房间和事件号：房间只对 IM 来的那一轮有意义。认不出开页的人（只读连接、没有记录）时
        # 留着来源，和认不出派发标记时一样按 IM 兜底。
        origin = {}
    sender = page or sender_id or origin.get("sender")
    _remember_for_listing(session_id, sender)
    if not sender or session_id in _S["reported"]:
        return None
    if page is None and not origin.get("room"):
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
    if verdict == "stopped":
        # 报到的链已经停了（唤醒标签还指着它，常见于主人刚按下停止）：去掉消息 id 再报一次，
        # 核心就按会话新开一条链。停了的链本来也不接新消息，叫回同样开新链。
        body["event_id"] = ""
        verdict, refusal, detail = _chain_start(body)
    if verdict == "retry":
        if dispatched is not None:
            # 下一次模型调用还按这条 IM 算。
            _S["dispatched"].setdefault(session_id, {})[sender] = dispatched
        return None
    _S["reported"].add(session_id)
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
    return {"action": "block", "message": _refusal_text(refusal, sender, detail)}


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


def _refusal_text(refusal, sender, detail=""):
    """核心的拒绝类别换成给说话人看的一句话。

    类别说不清的地方（``invalid_input`` 没给 refusal、``other`` 什么都可能是）把核心给的说明接在后面。
    会话挂在别的链上、卡片已登记这类核心错误走的是 ``other``，类型分类只在响应头 ``x-wonton-error``
    里，插件只读响应体，所以落到下面泛泛的那句。
    """
    lang = _language(sender)
    kind = refusal.get("type")
    if kind == "permission_denied":
        text = CHAIN_DENIED[lang]
        return f"{text}\n\n{detail}" if detail else text
    if kind == "limit_reached":
        return CHAIN_LIMIT[lang]
    if kind == "invalid_input" and refusal.get("refusal") == "self_call":
        return CHAIN_SELF[lang]
    if kind == "invalid_input" and refusal.get("refusal") == "loop":
        return CHAIN_LOOP[lang]
    text = CHAIN_OTHER[lang]
    return f"{text}\n\n{detail}" if detail else text


def on_session_reset(session_id=None, old_session_id=None, **_):
    """会话被重置（``/new`` 等）：旧会话的网页记录和派发标记没人再读，删掉。"""
    _S["dispatched"].pop(old_session_id or session_id, None)
    _pages().forget(old_session_id or session_id)


def on_session_end(session_id="", **_):
    """这一轮结束：清身份，报过的链报结束。"""
    if _S["ending"] and session_id not in _S["ending"]:
        logger.warning("一轮结束报不回去 session=%s，手里还留着 %r", session_id, sorted(_S["ending"]))
    _S["active"].pop(session_id, None)
    _S["webs"].pop(session_id, None)
    _S["page_warned"].discard(session_id)
    # 派发标记不清：先结束的这一轮（常是网页轮）不能把还没跑的 IM 那一轮吃掉，只有重置才清。
    _pages().forget(session_id)
    _S["unknown"].discard(session_id)
    _S["unknown"].discard("")
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
    """给 MCP 会话的每次请求带上这一轮的身份头和来源；清单（``tools/list``）另外先包上会话。

    插件注册时 Hermes 还没导 ``tools.mcp_tool_handlers``，第一条消息到时再包；包过以后这里是空的。
    """
    _wrap_list_tools()
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
        env = _session_id()
        turn, source = _identity(env)
        logger.debug("MCP 调用的身份取自%s：环境的会话 %r，带的 %r", source, env, turn)
        execution = _cron_execution()
        cron = await _cron_identity_async(execution) if execution is not None else None
        if cron is not None:
            turn, source = cron, "定时任务"
            _S["last"] = turn
            logger.debug("MCP 调用来自定时任务：替创建者 %s 说话，不带会话", turn["sender"])
        elif execution is not None and turn is not None:
            # 定时任务的会话号核心认不出，别拿它去猜别的轮。
            turn = {**turn, "session": None}
        if turn is None:
            if source != "轮次":
                logger.debug("MCP 调用的身份取自%s：环境的会话 %r，带的 %r", source, env, turn)
            if env not in _S["unknown"]:
                _S["unknown"].add(env)
                if env:
                    logger.warning("这一轮的身份取不到（环境的会话 %r）：调用不带代表头和会话", env)
                else:
                    logger.warning("这一轮没有会话号，身份取不到：调用不带代表头和会话")
            # 明确压一个空身份：别让盖章退回去拿上一轮的 last 猜这次调用。
            token = _CURRENT.set({})
            try:
                return await original(server, *args, **kwargs)
            finally:
                _CURRENT.reset(token)
        _S["last"] = turn
        _patch_session(server.session)
        token = _CURRENT.set(turn)
        try:
            return await original(server, *args, **kwargs)
        finally:
            _CURRENT.reset(token)

    call._wonton_turn = True
    mcp_tool_handlers._call_tool_racing_stdio_death = call


def _wrap_list_tools():
    """清单也带身份：会话一建好、还没有任何调用时就把它包上。

    清单是连接、重连时取的，那时没有进行中的调用，身份用这一轮在 ``_S["last"]`` 上放的那份。包在
    ``list_tools`` 外面，请求进来先包会话（``_stamp`` 在第一个 await 之前同步跑完，补进 ``opts`` 的头一定在
    发出去的请求里）。
    """
    try:
        from mcp.client.session import ClientSession
    except Exception as exc:
        if not _S["warned_list"]:
            _S["warned_list"] = True
            logger.warning("上游没有会话的 ``list_tools`` 可包，清单不会带身份：%r", exc)
        return
    if getattr(ClientSession.list_tools, "_wonton_list", False):
        return
    original = ClientSession.list_tools

    async def list_tools(self, *args, **kwargs):
        _patch_session(self)
        return await original(self, *args, **kwargs)

    list_tools._wonton_list = True
    ClientSession.list_tools = list_tools


def _stamp_turn(session):
    """让会话的每次请求都带这一轮的身份。

    上游把 resolve_param_headers 在 ``adopt()`` 时就包进 ``_stamp`` 闭包，之后再换 ``_resolve_param_headers``
    不起作用；``_stamp`` 每次请求现调，改它才落到线上。``adopt()`` 会换掉 ``_stamp``，包过以后跟着重包。

    身份按序取：工具线程里这次调用 set 的 ``_CURRENT``；取不到用这一轮在 ``_S["last"]`` 上放的
    （``_run_on_mcp_loop`` 拷的是循环线程的 context，``_CURRENT`` 进不了清单）。
    """
    stamp = session._stamp
    if getattr(stamp, "_wonton_stamp", False):
        return

    def with_turn(data, opts):
        stamp(data, opts)
        turn = _CURRENT.get()
        if turn is None:
            turn = _S["last"]
        if not turn:
            return
        headers = opts.setdefault("headers", {})
        if turn.get("sender"):
            headers[ON_BEHALF_OF] = turn["sender"]
        if turn.get("session"):
            headers[SESSION] = turn["session"]

    with_turn._wonton_stamp = True
    session._stamp = with_turn


def _patch_session(session):
    if getattr(session, "_wonton_turn", False):
        return
    session._wonton_turn = True
    _stamp_turn(session)
    original_adopt = session.adopt

    def adopt(result):
        original_adopt(result)
        _stamp_turn(session)

    session.adopt = adopt
    original_call = session.call_tool

    async def call_tool(name, arguments=None, *args, **kwargs):
        turn = _CURRENT.get()
        if turn and "meta" not in kwargs:
            origin = {name: turn[key] for key, name in (("room", "room_id"), ("thread", "thread_id")) if turn.get(key)}
            if origin:
                # 核心的 origin 要求 room_id；只有会话号时塞空的 {} 反而解析不了，不带。
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


def _release_slot(runner, session_key):
    """打断这一轮但不依赖它的清会话钩子：只放槽，走在跑的上游驱逐同一对调用。"""
    generation = runner._interrupt_running_turn(
        session_key,
        interrupt_reason="云吞停链",
        invalidation_reason="wonton_chain_stopped",
    )
    runner._drop_turn_slot(session_key, run_generation=generation)


async def _do_outbox(item, client=None):
    item_id = item.get("id")
    kind = item.get("kind") or {}
    if kind.get("type") == "cancel_turn":
        session = kind.get("session") or ""
        origin = _origin(session) or {}
        session_key = origin.get("session_key")
        source = origin.get("source")
        runner = _runner()
        if session_key and runner is not None:
            clear = getattr(runner, "_interrupt_and_clear_session", None)
            try:
                if clear is not None and source is not None:
                    # 和 /stop 走同一条路：光打断不停用那一轮的槽，槽一直占着，之后的消息全被当成“忙”排队，没人放。
                    await clear(
                        session_key,
                        source,
                        interrupt_reason="云吞停链",
                        invalidation_reason="wonton_chain_stopped",
                    )
                else:
                    _release_slot(runner, session_key)
            except Exception as exc:
                # 清会话时抛了异常也要把槽放掉；核心不重发 CancelTurn，槽占着就再也轮不到这一会话。
                logger.warning("停链时清会话失败 %s：%r", session, exc)
                try:
                    _release_slot(runner, session_key)
                except Exception as slot_exc:
                    logger.warning("停链时放轮次槽也失败 %s：%r", session, slot_exc)
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
