import json
import os
import tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DATA = Path("/data/notes.json")
PAGE = Path("/app/index.html").read_bytes()
MAX_BODY = 64 * 1024


def load():
    return json.loads(DATA.read_text()) if DATA.exists() else []


def save(notes):
    # 先写临时文件再改名：写到一半被停掉时，旧数据还在。
    fd, tmp = tempfile.mkstemp(dir=DATA.parent)
    with os.fdopen(fd, "w") as f:
        json.dump(notes, f, ensure_ascii=False)
    os.replace(tmp, DATA)


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body=b"", kind="application/json"):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/healthz":
            self.reply(200, b"ok", "text/plain")
        elif self.path == "/api/notes":
            self.reply(200, json.dumps(load(), ensure_ascii=False).encode())
        elif self.path == "/":
            self.reply(200, PAGE, "text/html; charset=utf-8")
        else:
            self.reply(404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        if self.path != "/api/notes" or length > MAX_BODY:
            self.reply(400)
            return
        try:
            text = json.loads(self.rfile.read(length))["text"].strip()
        except (ValueError, KeyError, TypeError, AttributeError):
            self.reply(400)
            return
        if not text:
            self.reply(400)
            return
        notes = load() + [text]
        save(notes)
        self.reply(200, json.dumps(notes, ensure_ascii=False).encode())


ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
