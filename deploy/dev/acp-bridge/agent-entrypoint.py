#!/usr/bin/env python3
import asyncio
import json
import os
import signal
import socket
from pathlib import Path


SOCKET = Path(os.environ.get("WONTON_ACP_SOCKET", "/run/wonton/acp.sock"))
WORKSPACE = Path(os.environ.get("WONTON_AGENT_WORKSPACE", "/workspace"))
AGENT_HOME = Path(os.environ.get("HOME", "/data/agent/home"))
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
    # OMP 不读取 OPENAI_BASE_URL/OPENAI_MODEL；密钥只引用环境变量名。
    model = os.environ["OPENAI_MODEL"]
    base_url = os.environ["OPENAI_BASE_URL"]
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


async def main() -> None:
    own_tree(AGENT_HOME.parent)
    own_tree(WORKSPACE)
    own_tree(SOCKET.parent)
    if os.geteuid() == 0:
        os.setgroups([])
        os.setgid(AGENT_GID)
        os.setuid(AGENT_UID)
    configure_model()
    try:
        SOCKET.unlink()
    except FileNotFoundError:
        pass
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(SOCKET))
    os.chmod(SOCKET, 0o600)
    listener.listen(8)
    listener.setblocking(False)
    process = await asyncio.create_subprocess_exec(
        "/usr/local/bin/omp",
        "acp",
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
        listener.close()
        try:
            SOCKET.unlink()
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    asyncio.run(main())
