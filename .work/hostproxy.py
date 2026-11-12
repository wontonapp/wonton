#!/usr/bin/env python3
"""本机小代理：接 HTTP 代理请求，转发到 127.0.0.1:19680，Host 改写成网关认的 matrix 主机名。"""
import socket, threading
import os
LISTEN = ("127.0.0.1", int(os.environ.get("PORT", "19681")))
TARGET = (lambda h, p: (h, int(p)))(*os.environ["TARGET"].split(":"))
HOST = os.environ["FAKEHOST"].encode()

def pipe(a, b):
    try:
        while (d := a.recv(65536)):
            b.sendall(d)
    except OSError:
        pass
    finally:
        try: b.shutdown(socket.SHUT_WR)
        except OSError: pass

def handle(c):
    try:
        buf = b""
        while b"\r\n\r\n" not in buf:
            d = c.recv(65536)
            if not d: return
            buf += d
        head, rest = buf.split(b"\r\n\r\n", 1)
        lines = head.split(b"\r\n")
        first = lines[0].split(b" ")
        if first[1].startswith(b"http://"):
            from urllib.parse import urlsplit
            u = urlsplit(first[1].decode())
            first[1] = (u.path or "/") + (("?" + u.query) if u.query else "")
            first[1] = first[1].encode()
        out = [b" ".join(first)]
        for line in lines[1:]:
            if line.lower().startswith(b"host:"):
                continue
            out.append(line)
        out.append(b"Host: " + HOST)
        up = socket.create_connection(TARGET)
        up.sendall(b"\r\n".join(out) + b"\r\n\r\n" + rest)
        threading.Thread(target=pipe, args=(up, c), daemon=True).start()
        pipe(c, up)
    except OSError:
        pass
    finally:
        c.close()

s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(LISTEN); s.listen(64)
while True:
    conn, _ = s.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
