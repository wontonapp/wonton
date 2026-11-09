#!/usr/bin/env python3
"""C3 登录验收的测试 ACP Agent：无外部账号，只验证桥的登录链路。

两种模式：
- 默认：ACP stdio Agent。session/new 在登录状态文件不存在时返回 -32000；
  authenticate(test-url) 走 elicitation/create URL 引导，收到 accept 后写状态文件。
- --login：终端登录命令。打印授权链接，从 stdin 读一行授权码，写状态文件，退出 0。

登录状态文件在 $HOME 下，随 Agent 数据卷持久化。仅供部署验收，不是产品代码。
"""
import json
import os
import sys
from pathlib import Path

STATE = Path(os.environ.get("HOME", "/tmp")) / ".c3-login-ok"
URL_METHOD = "test-url"
TERMINAL_METHOD = "test-terminal"


def logged_in() -> bool:
    return STATE.exists()


def grant_login() -> None:
    STATE.parent.mkdir(parents=True, exist_ok=True)
    STATE.write_text("ok")


def run_agent() -> None:
    next_id = 100
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = message.get("method", "")
        if method == "initialize":
            respond(message["id"], {
                "protocolVersion": 1,
                "agentCapabilities": {"mcpCapabilities": {"http": {}}},
                "authMethods": [
                    {"id": URL_METHOD, "name": "测试 URL 登录"},
                    {
                        "id": TERMINAL_METHOD,
                        "name": "测试终端登录",
                        "type": "terminal",
                        "args": ["--login"],
                    },
                ],
            })
        elif method == "session/new":
            if logged_in():
                respond(message["id"], {"sessionId": "c3-session"})
            else:
                error(message["id"], -32000, "Authentication required")
        elif method == "session/prompt":
            notify_update("C3 登录后的测试回复")
            respond(message["id"], {"stopReason": "end_turn"})
        elif method == "authenticate":
            if message["params"]["methodId"] != URL_METHOD:
                error(message["id"], -32601, "Unknown auth method")
                continue
            next_id += 1
            elicit_id = next_id
            request(elicit_id, "elicitation/create", {
                "mode": "url",
                "elicitationId": "c3-login",
                "url": "https://auth.example/c3/device",
                "message": "打开链接完成测试登录",
            })
            answer = wait_response(elicit_id)
            if answer == "accept":
                grant_login()
                respond(message["id"], {})
            else:
                error(message["id"], -32000, "登录未完成")
        elif method == "session/set_mode":
            respond(message["id"], {})
        else:
            error(message.get("id", 0), -32601, "Method not found")


def wait_response(request_id: int) -> str:
    for line in sys.stdin:
        message = json.loads(line)
        if message.get("id") == request_id and "result" in message:
            return message["result"].get("action", "cancel")
    return "cancel"


def run_login() -> None:
    print("https://auth.example/c3/device 输入本行作为授权码", flush=True)
    code = sys.stdin.readline().strip()
    if not code:
        sys.exit(2)
    grant_login()
    print("登录完成", flush=True)


def respond(request_id, result) -> None:
    write({"jsonrpc": "2.0", "id": request_id, "result": result})


def error(request_id, code, message) -> None:
    write({"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}})


def request(request_id, method, params) -> None:
    write({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})


def notify_update(text: str) -> None:
    write({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": "c3-session",
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "messageId": "c3-msg",
            "content": {"type": "text", "text": text},
        },
    }})


def write(message: dict) -> None:
    sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
    sys.stdout.flush()


if __name__ == "__main__":
    if "--login" in sys.argv[1:]:
        run_login()
    else:
        run_agent()
