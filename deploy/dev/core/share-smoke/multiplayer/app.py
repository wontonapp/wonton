"""分享冒烟用的多人房间：一条 WebSocket 一个玩家，所有人共用同一个房间。

网关按人注入 X-Remote-*，应用层伪造的同名头一律看不到（网关换掉）。
"""

import base64
import hashlib
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
PLAYERS = {}
LOCK = threading.Lock()


def accept(key):
    return base64.b64encode(hashlib.sha1(key + GUID).digest()).decode()


def read_frame(rfile):
    first, second = rfile.read(2)
    length = second & 0x7F
    if length == 126:
        length = int.from_bytes(rfile.read(2), "big")
    elif length == 127:
        length = int.from_bytes(rfile.read(8), "big")
    mask = rfile.read(4) if second & 0x80 else b""
    payload = bytearray()
    for index in range(length):
        byte = rfile.read(1)
        if not byte:
            return None
        payload.append(byte[0] ^ mask[index % 4])
    return first & 0x0F, bytes(payload)


def send(wfile, opcode, payload):
    header = bytes([0x80 | opcode])
    if len(payload) < 126:
        header += bytes([len(payload)])
    else:
        header += bytes([126]) + len(payload).to_bytes(2, "big")
    wfile.write(header + payload)


def identity(headers, number):
    return {
        "user": headers.get("X-Remote-User", ""),
        "name": headers.get("X-Remote-Name", ""),
        "role": headers.get("X-Remote-Role", ""),
        "number": number,
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path.split("?")[0] == "/ws":
            self.websocket()
        else:
            self.page()

    # 核心探测授权只发 HEAD，页面要能答它。
    def do_HEAD(self):
        self.page(body=False)

    def page(self, body=True):
        content = b"<!doctype html><meta charset=utf-8><title>Share smoke</title><p>multiplayer"
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        if body:
            self.wfile.write(content)

    def websocket(self):
        key = self.headers.get("Sec-WebSocket-Key", "").encode()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept(key))
        self.end_headers()
        with LOCK:
            number = len(PLAYERS) + 1
        with LOCK:
            PLAYERS[self.connection] = identity(self.headers, number)
        try:
            send(self.wfile, 1, json.dumps({
                "identity": identity(self.headers, number),
                "players": list(PLAYERS.values()),
            }).encode())
            while True:
                frame = read_frame(self.rfile)
                if frame is None:
                    return
                opcode, payload = frame
                if opcode == 8:
                    return
                send(self.wfile, opcode, payload)
        except OSError:
            return
        finally:
            with LOCK:
                PLAYERS.pop(self.connection, None)


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 8000), Handler).serve_forever()
