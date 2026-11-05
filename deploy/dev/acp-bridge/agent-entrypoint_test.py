#!/usr/bin/env python3
"""agent-entrypoint.py 登录转发的行为测试：python3 -m unittest 或直接运行。"""
import asyncio
import importlib.util
import json
import os
import socket
import sys
import tempfile
import time
import unittest
from pathlib import Path

HERE = Path(__file__).parent


def load_entrypoint():
    spec = importlib.util.spec_from_file_location("agent_entrypoint", HERE / "agent-entrypoint.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ENTRY = load_entrypoint()
BLOCKING = [sys.executable, "-c", "import sys; sys.stdin.read()"]


def nonblocking_pair():
    a, b = socket.socketpair()
    a.setblocking(False)
    b.setblocking(False)
    return a, b


def wait_gone(pid: int, timeout: float = 3.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not Path(f"/proc/{pid}").exists():
            return True
        time.sleep(0.05)
    return False


class RelayLoginTest(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        ENTRY.WORKSPACE = HERE
    async def test_socket_disconnect_ends_waiting_login(self):
        """桥断开时登录进程还堵在 stdin：转发必须结束并杀掉它，下一次登录能开始。"""
        loop = asyncio.get_running_loop()
        pid_file = Path(tempfile.mkdtemp()) / "pid"
        blocking = [
            sys.executable,
            "-c",
            "import os,sys;open(sys.argv[1],'w').write(str(os.getpid()));sys.stdin.read()",
            str(pid_file),
        ]
        first, server = nonblocking_pair()
        task = asyncio.create_task(ENTRY.relay_login(server, loop, command=blocking))
        await loop.sock_sendall(first, b'{"args":[],"env":{}}\n')
        for _ in range(100):
            if pid_file.exists():
                break
            await asyncio.sleep(0.02)
        pid = int(pid_file.read_text())

        # 桥侧写端关闭 = 主人取消/桥退出。
        first.shutdown(socket.SHUT_WR)
        await asyncio.wait_for(task, 5)
        self.assertTrue(wait_gone(pid), "登录进程没有被结束")
        server.close()

        # 同一路服务能再跑下一次登录。
        second, server2 = nonblocking_pair()
        task = asyncio.create_task(ENTRY.relay_login(server2, loop, command=BLOCKING))
        await loop.sock_sendall(second, b'{"args":[],"env":{}}\n')
        await asyncio.sleep(0.2)
        second.shutdown(socket.SHUT_WR)
        await asyncio.wait_for(task, 5)
        second.close()
        server2.close()

    async def test_streams_reach_the_client(self):
        """stdout 和 stderr 每行都转给桥。"""
        loop = asyncio.get_running_loop()
        command = [
            sys.executable,
            "-c",
            "import sys;print('out-line',flush=True);print('err-line',file=sys.stderr,flush=True)",
        ]
        client, server = nonblocking_pair()
        task = asyncio.create_task(ENTRY.relay_login(server, loop, command=command))
        await loop.sock_sendall(client, b'{"args":[],"env":{}}\n')

        frames = []

        async def pump():
            while line := await loop.sock_recv(client, 4096):
                for chunk in line.decode().splitlines():
                    frames.append(json.loads(chunk))
                if any("exit" in frame for frame in frames):
                    return

        await asyncio.wait_for(pump(), 5)
        await asyncio.wait_for(task, 5)
        outs = [frame.get("out") for frame in frames if "out" in frame]
        errs = [frame.get("err") for frame in frames if "err" in frame]
        self.assertIn("out-line", outs)
        self.assertIn("err-line", errs)
        self.assertEqual(frames[-1].get("exit"), 0)
        client.close()
        server.close()


if __name__ == "__main__":
    os.chdir(HERE)
    unittest.main()
