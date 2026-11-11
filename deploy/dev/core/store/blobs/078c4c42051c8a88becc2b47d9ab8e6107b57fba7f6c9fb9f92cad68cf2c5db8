"""回消息用的语言，按人取。

App 把自己的语言放进每个请求的 Accept-Language，核心按人记下；这里问核心的 wonton__user_locale 工具。
问不到、这个人还没有语言记录、他用的语言不在支持范围内时退到主人，再退英文（App 的基础语言）：
按英文回也比按猜错的语言回好懂。

只在这条消息要用提示时才问，同一个人的结果（含查不到）缓存几分钟：还没加模型时会连着发几条，不必每条都问。

总线地址 WONTON_MCP_URL 带这个 Bot 的令牌。按 MCP 的 Streamable HTTP 走：initialize 拿会话、
notifications/initialized、tools/call，最后 DELETE 掉会话，只有这一次调用用它。

总线在插件网络里，不经过出网代理，所以客户端不读代理环境变量（Hermes 自己的客户端也一样）。
"""

import asyncio
import contextlib
import json
import logging
import os
import time

import httpx

logger = logging.getLogger(__name__)

# 提示有这四种语言，和 App 支持的四种一致；都不支持时英文。
SUPPORTED = ("en", "zh-Hans", "zh-Hant", "ja")
# 主人改了语言，过几分钟跟上。
CACHE_TTL = 300
TIMEOUT = 3.0

# 人（矩阵 ID，空串是主人）→ (过期时刻, 语言列表)。
_cache: dict[str, tuple[float, list[str]]] = {}


def choose(tags) -> str:
    """按偏好顺序挑一条支持的语言；一条都不支持时英文。"""
    for tag in tags:
        parts = str(tag).split("-")
        base, subtags = parts[0].lower(), {part.lower() for part in parts[1:]}
        if base in SUPPORTED:
            return base
        if base == "zh":
            return "zh-Hant" if "hant" in subtags or subtags & {"tw", "hk", "mo"} else "zh-Hans"
    return "en"


async def language(user_id: str | None) -> str:
    """这个人说话用的语言；取不到时用主人的，再取不到英文。"""
    tags = await _tags(user_id)
    if not tags and user_id and not _is_owner(user_id):
        tags = await _tags(None)
    return choose(tags)


def _is_owner(user_id: str) -> bool:
    """主人自己不用再退一级。没有 WONTON_OWNER_NAME（没申请 owner.identity）时一律当不是主人。"""
    owner = os.environ.get("WONTON_OWNER_NAME", "").strip()
    return bool(owner) and user_id.startswith(f"@{owner}:")


async def _tags(user_id: str | None):
    key = user_id or ""
    now = time.monotonic()
    cached = _cache.get(key)
    if cached and cached[0] > now:
        return cached[1]
    tags = await _ask(user_id)
    if len(_cache) >= 64:
        _cache.clear()
    _cache[key] = (now + CACHE_TTL, tags)
    return tags


async def _ask(user_id: str | None):
    url = os.environ.get("WONTON_MCP_URL", "")
    if not url:
        return []
    try:
        async with asyncio.timeout(TIMEOUT):
            async with httpx.AsyncClient(timeout=TIMEOUT, trust_env=False) as client:
                session = await _connect(client, url)
                try:
                    result = await _call(client, url, session, {"user_id": user_id} if user_id else {})
                finally:
                    with contextlib.suppress(httpx.HTTPError, TimeoutError):
                        await client.delete(url, headers={"Mcp-Session-Id": session})
                return _read(result)
    except (TimeoutError, httpx.HTTPError, ValueError, KeyError, TypeError) as error:
        logger.warning("问核心 %s 的语言失败，按英文回：%r", user_id or "主人", error)
        return []


async def _connect(client: httpx.AsyncClient, url: str) -> str:
    _, headers = await _post(
        client,
        url,
        _rpc(
            0,
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "wonton", "version": "1"},
            },
        ),
        None,
        0,
    )
    session = headers.get("mcp-session-id", "")
    if not session:
        raise ValueError("initialize 没给会话")
    await _post(client, url, {"jsonrpc": "2.0", "method": "notifications/initialized"}, session, None)
    return session


async def _call(client: httpx.AsyncClient, url: str, session: str, arguments: dict):
    result, _ = await _post(
        client,
        url,
        _rpc(1, "tools/call", {"name": "wonton__user_locale", "arguments": arguments}),
        session,
        1,
    )
    return result


async def _post(client: httpx.AsyncClient, url: str, body: dict, session: str | None, request_id):
    """发一条消息，返回 (结果, 响应头)；通知没有 id，不读结果。"""
    headers = {"Accept": "application/json, text/event-stream"}
    if session:
        headers["Mcp-Session-Id"] = session
    async with client.stream("POST", url, headers=headers, json=body) as response:
        response.raise_for_status()
        if request_id is None:
            return None, response.headers
        return await _result(response, request_id), response.headers


async def _result(response: httpx.Response, request_id):
    """结果可能是一整条 JSON，也可能是 SSE 里带 id 的那条消息，其它事件跳过。"""
    if response.headers.get("content-type", "").startswith("application/json"):
        await response.aread()
        return json.loads(response.text).get("result") or {}
    async for line in response.aiter_lines():
        payload = line[5:].strip() if line.startswith("data:") else ""
        # 流开头有一条只有 id 和 retry 的空事件。
        if not payload:
            continue
        message = json.loads(payload)
        if message.get("id") == request_id:
            return message.get("result") or {}
    raise ValueError("没等到工具结果")


def _read(result) -> list[str]:
    """工具结果里的语言列表；工具报错时记下原因，当没有记录。"""
    text = next(
        (block.get("text") or "" for block in result.get("content") or [] if block.get("type") == "text"),
        "",
    )
    if result.get("isError"):
        logger.warning("核心没给这个人的语言：%s", text)
        return []
    return [str(tag) for tag in json.loads(text).get("languages") or []]


def _rpc(request_id: int, method: str, params: dict) -> dict:
    return {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
