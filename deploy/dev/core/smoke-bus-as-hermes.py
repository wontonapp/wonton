#!/usr/bin/env python3
"""smoke-turns.sh 的说话人一步在 Hermes 的容器里跑它：以 Hermes 的身份直接问核心，不经模型。

  tools <说话人|-> <会话|->         MCP 总线给的工具列表里有没有 wonton__save_file，输出 yes 或 no。说话人放进
                                    X-Wonton-On-Behalf-Of，会话放进 wonton-session，`-` 表示不带。只读封顶的人拿不到
                                    这个工具，所以它说明核心这次按谁的权限算。
  start <会话> <说话人> <房间>      用 Bot 的 Matrix 令牌报一轮开始（和 Hermes 的 wonton 插件报的一样），输出链 ID。
  end <会话>                        报这一轮结束。

用 Hermes 镜像里的 httpx，不经代理：核心在插件网络里。
"""

import json
import os
import sys

import httpx

HEAD, _, TOKEN = os.environ["WONTON_MCP_URL"].rpartition("/mcp/")
MCP = f"{HEAD}/mcp/{TOKEN}"
CHAINS = f"{HEAD}/agent/chains/{TOKEN}"
BOT = {"X-Wonton-Bot-Token": os.environ["MATRIX_ACCESS_TOKEN"]}


def body_of(response):
    """JSON-RPC 的回答：总线按请求的 Accept 回 JSON 或一条 SSE 事件。"""
    response.raise_for_status()
    if response.headers.get("content-type", "").startswith("text/event-stream"):
        for line in response.text.splitlines():
            if line.startswith("data:"):
                return json.loads(line[5:])
        sys.exit(f"SSE 响应里没有数据：{response.text[:300]}")
    return response.json()


def offers_save(client, speaker, session):
    headers = {"accept": "application/json, text/event-stream"}
    if speaker != "-":
        headers["X-Wonton-On-Behalf-Of"] = speaker
    if session != "-":
        headers["wonton-session"] = session
    init = client.post(MCP, headers=headers, json={
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "wonton-smoke", "version": "0"}},
    })
    result = body_of(init)["result"]
    if "mcp-session-id" in init.headers:
        headers["mcp-session-id"] = init.headers["mcp-session-id"]
    headers["mcp-protocol-version"] = result["protocolVersion"]
    client.post(MCP, headers=headers,
                json={"jsonrpc": "2.0", "method": "notifications/initialized"}).raise_for_status()
    names, cursor = set(), None
    while True:
        params = {"cursor": cursor} if cursor else {}
        page = body_of(client.post(MCP, headers=headers, json={
            "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": params,
        }))["result"]
        names.update(tool["name"] for tool in page["tools"])
        cursor = page.get("nextCursor")
        if not cursor:
            return "yes" if "wonton__save_file" in names else "no"


def main(args):
    with httpx.Client(trust_env=False, timeout=60) as client:
        if args[0] == "tools":
            print(offers_save(client, args[1], args[2]))
        elif args[0] == "start":
            session, sender, room = args[1:4]
            report = {"phase": "start", "session": session, "room": room, "sender": sender,
                      "event_id": f"$smoke-{session}"}
            response = client.post(CHAINS, headers=BOT, json=report)
            if response.status_code != 200:
                sys.exit(f"报开始被拒：HTTP {response.status_code} {response.text[:300]}")
            print(response.json()["chain"])
        elif args[0] == "end":
            client.post(CHAINS, headers=BOT, json={"phase": "end", "session": args[1]}).raise_for_status()
        else:
            sys.exit(f"不认识的命令：{args[0]}")


if __name__ == "__main__":
    main(sys.argv[1:])
