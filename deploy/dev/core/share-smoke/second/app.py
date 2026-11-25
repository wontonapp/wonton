"""分享冒烟的第二个插件：只答一页，证明分享连接只到授权过的插件。"""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        self.page()

    def do_HEAD(self):
        self.page(body=False)

    def page(self, body=True):
        content = b"<!doctype html><meta charset=utf-8><title>Share smoke two</title><p>second"
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        if body:
            self.wfile.write(content)


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 8000), Handler).serve_forever()
