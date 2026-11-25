#!/usr/bin/env python3
"""Serves the test page, hands out LiveKit tokens and appends page logs to a file.

Usage: serve.py <site-dir> <log-file>
LiveKit dev key/secret are fixed by `livekit-server --dev` (devkey/secret).
"""
import base64
import hashlib
import hmac
import json
import sys
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

SITE, LOG = sys.argv[1], sys.argv[2]


def b64(raw: bytes) -> bytes:
    return base64.urlsafe_b64encode(raw).rstrip(b"=")


def token(room: str, identity: str) -> str:
    now = int(time.time())
    head = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    body = b64(json.dumps({
        "exp": now + 3600, "iss": "devkey", "sub": identity, "nbf": now - 10,
        "video": {"room": room, "roomJoin": True},
    }, separators=(",", ":")).encode())
    sig = b64(hmac.new(b"secret", head + b"." + body, hashlib.sha256).digest())
    return (head + b"." + body + b"." + sig).decode()


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=SITE, **kw)

    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/token":
            q = parse_qs(url.query)
            raw = token(q.get("room", ["calltest"])[0], q.get("identity", ["webview"])[0])
            self._send(200, "application/json", json.dumps({"token": raw}).encode())
            return
        super().do_GET()

    def do_POST(self):
        if urlparse(self.path).path != "/log":
            self.send_error(404)
            return
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        line = body.decode(errors="replace").replace("\n", " | ")
        with open(LOG, "a") as f:
            f.write("%.3f %s\n" % (time.time(), line))
            f.flush()
        self._send(204, "text/plain", b"")

    def _send(self, code, ctype, body):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)


ThreadingHTTPServer(("127.0.0.1", 8000), Handler).serve_forever()
