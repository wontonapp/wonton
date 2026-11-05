#!/usr/bin/env python3
import asyncio
import json
import shlex
import os
import signal
import socket
from pathlib import Path


SOCKET = Path(os.environ.get("WONTON_ACP_SOCKET", "/run/wonton/acp.sock"))
WORKSPACE = Path(os.environ.get("WONTON_AGENT_WORKSPACE", "/workspace"))
AGENT_HOME = Path(os.environ.get("HOME", "/data/agent/home"))
AGENT_COMMAND = os.environ.get("WONTON_AGENT_COMMAND", "/usr/local/bin/omp acp")
AGENT_UID = 65532
AGENT_GID = 65532


def own_tree(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    for root, dirs, files in os.walk(path):
        os.chown(root, AGENT_UID, AGENT_GID)
        for name in dirs + files:
            os.chown(Path(root, name), AGENT_UID, AGENT_GID, follow_symlinks=False)


async def relay(client: socket.socket, process: asyncio.subprocess.Process) -> None:
    loop = asyncio.get_running_loop()

    async def socket_to_agent() -> None:
        while True:
            data = await loop.sock_recv(client, 512 * 1024)
            if not data:
                return
            if process.stdin:
                process.stdin.write(data)
                await process.stdin.drain()

    async def agent_to_socket() -> None:
        while True:
            if process.stdout is None:
                return
            data = await process.stdout.read(512 * 1024)
            if not data:
                return
            await loop.sock_sendall(client, data)

    tasks = [asyncio.create_task(socket_to_agent()), asyncio.create_task(agent_to_socket())]
    done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
    for task in pending:
        task.cancel()
    for task in done:
        task.result()


def configure_model() -> None:
    # OMP 不读取 OPENAI_BASE_URL/OPENAI_MODEL；密钥只引用环境变量名。不设模型变量时跳过。
    model = os.environ.get("OPENAI_MODEL", "")
    base_url = os.environ.get("OPENAI_BASE_URL", "")
    if not model and not base_url:
        return
    if not model.strip() or not base_url.strip() or not os.environ["OPENAI_API_KEY"].strip():
        raise ValueError("OPENAI_MODEL、OPENAI_BASE_URL 和 OPENAI_API_KEY 不能为空")
    directory = AGENT_HOME / ".omp" / "agent"
    directory.mkdir(parents=True, exist_ok=True)
    provider = {
        "baseUrl": base_url,
        "api": "openai-completions",
        "apiKey": "OPENAI_API_KEY",
        "models": [{"id": model}],
    }
    (directory / "models.yml").write_text(json.dumps({"providers": {"wonton": provider}}))
    selector = f"wonton/{model}"
    settings = {"modelRoles": {role: selector for role in ("default", "smol", "slow", "plan")}}
    (directory / "config.yml").write_text(json.dumps(settings))


async def serve_login(listener: socket.socket) -> None:
    """终端类登录：桥连这个 socket，入口用同一 Agent 程序启动登录进程并转发 stdio。

    协议（与桥 login/terminal.rs 约定）：桥先发一行 JSON {"methodId","args","env"}；
    之后桥发的每行文本写入登录进程 stdin；登录进程的 stdout/stderr 每行回 {"out"}/{"err"}，
    结束时回 {"exit": 数字}。描述符的 env 只作用于这次登录进程。
    """
    loop = asyncio.get_running_loop()
    while True:
        client, _ = await loop.sock_accept(listener)
        try:
            await relay_login(client, loop)
        finally:
            client.close()


async def relay_login(client: socket.socket, loop: asyncio.AbstractEventLoop, command=None) -> None:
    reader = asyncio.StreamReader()
    await loop.connect_read_pipe(lambda: asyncio.StreamReaderProtocol(reader), client.makefile("rb"))
    command = command or shlex.split(AGENT_COMMAND)
    request_line = await reader.readline()
    if not request_line:
        return
    request = json.loads(request_line)
    process = await asyncio.create_subprocess_exec(
        *command, *request.get("args", []),
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, **request.get("env", {})},
        cwd=WORKSPACE,
    )
    stdin = process.stdin

    async def to_socket(frame: dict) -> None:
        await loop.sock_sendall(client, (json.dumps(frame, ensure_ascii=False) + "\n").encode())

    async def pump_output(stream, key) -> None:
        assert stream is not None
        while line := await stream.readline():
            text = line.decode(errors="replace").rstrip("\n")
            await to_socket({key: text})

    async def pump_input() -> None:
        assert stdin is not None
        while line := await reader.readline():
            stdin.write(line)
            await stdin.drain()

    stdout_task = asyncio.create_task(pump_output(process.stdout, "out"))
    stderr_task = asyncio.create_task(pump_output(process.stderr, "err"))
    input_task = asyncio.create_task(pump_input())
    exit_task = asyncio.create_task(process.wait())
    done, _ = await asyncio.wait({exit_task, input_task}, return_when=asyncio.FIRST_COMPLETED)
    if input_task in done:
        # 桥断开（取消、超时、桥退出）：登录进程可能还在等 stdin，必须结束并收回，
        # 否则串行的 serve_login 会一直卡在这次登录上。
        stdin.close()
        try:
            code = await asyncio.wait_for(process.wait(), 2)
        except asyncio.TimeoutError:
            process.terminate()
            try:
                code = await asyncio.wait_for(process.wait(), 2)
            except asyncio.TimeoutError:
                process.kill()
                code = await process.wait()
    else:
        code = exit_task.result()
    input_task.cancel()
    await asyncio.gather(stdout_task, stderr_task, exit_task, return_exceptions=True)
    try:
        await to_socket({"exit": code})
    except OSError:
        pass


async def main() -> None:
    own_tree(AGENT_HOME.parent)
    own_tree(WORKSPACE)
    own_tree(SOCKET.parent)
    if os.geteuid() == 0:
        os.setgroups([])
        os.setgid(AGENT_GID)
        os.setuid(AGENT_UID)
    configure_model()
    login_socket = SOCKET.with_suffix(".login.sock")
    for path in (SOCKET, login_socket):
        try:
            path.unlink()
        except FileNotFoundError:
            pass
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(SOCKET))
    os.chmod(SOCKET, 0o600)
    listener.listen(8)
    listener.setblocking(False)
    login_listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    login_listener.bind(str(login_socket))
    os.chmod(login_socket, 0o600)
    login_listener.listen(2)
    login_listener.setblocking(False)
    login_task = asyncio.create_task(serve_login(login_listener))
    command = shlex.split(AGENT_COMMAND)
    process = await asyncio.create_subprocess_exec(
        *command,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=None,
        cwd=WORKSPACE,
    )
    loop = asyncio.get_running_loop()
    try:
        while process.returncode is None:
            accept_task = asyncio.create_task(loop.sock_accept(listener))
            exit_task = asyncio.create_task(process.wait())
            done, pending = await asyncio.wait(
                [accept_task, exit_task],
                return_when=asyncio.FIRST_COMPLETED,
            )
            for task in pending:
                task.cancel()
            if exit_task in done:
                break
            client, _ = accept_task.result()
            client.setblocking(False)
            try:
                await relay(client, process)
            finally:
                client.close()
    finally:
        if process.returncode is None:
            process.send_signal(signal.SIGTERM)
        await process.wait()
        login_task.cancel()
        await asyncio.gather(login_task, return_exceptions=True)
        listener.close()
        login_listener.close()
        for path in (SOCKET, login_socket):
            try:
                path.unlink()
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    asyncio.run(main())
