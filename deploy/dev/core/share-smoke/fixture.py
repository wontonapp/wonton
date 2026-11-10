#!/usr/bin/env python3
"""Authoritative .test DNS and TLS entrance for the isolated sharing smoke."""
import http.client
import json
import os
from pathlib import Path
import socket
import socketserver
import ssl
import struct
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

STATE = Path(os.environ["SHARE_SMOKE_STATE"])
PREFIX = os.environ["SHARE_SMOKE_PREFIX"]
IDENTITY_PREFIX = os.environ["SHARE_SMOKE_IDENTITY_PREFIX"]


def exact(stream, length):
    data = bytearray()
    while len(data) < length:
        part = stream.recv(length - len(data))
        if not part:
            raise EOFError
        data.extend(part)
    return bytes(data)


def dns_answer(packet):
    if len(packet) < 12 or struct.unpack("!H", packet[4:6])[0] != 1:
        return b""
    pos, labels = 12, []
    while packet[pos]:
        size = packet[pos]
        if size > 63:
            return b""
        labels.append(packet[pos + 1:pos + size + 1].decode("ascii").lower())
        pos += size + 1
    pos += 1
    kind, cls = struct.unpack("!HH", packet[pos:pos + 4])
    question = packet[12:pos + 4]
    name = ".".join(labels)
    addresses = {"dns.test": 2, "pebble.test": 3, "relay.test": 12,
                 "owner.test": 10, "friend.test": 10, "external.test": 10,
                 "mismatch.test": 10, "bad-cert.test": 11}
    answer = b""
    if cls == 1 and kind == 1 and name in addresses:
        prefix = IDENTITY_PREFIX if name in {"external.test", "mismatch.test", "bad-cert.test"} else PREFIX
        value = socket.inet_aton(f"{prefix}.{addresses[name]}")
        answer = b"\xc0\x0c" + struct.pack("!HHIH", kind, 1, 1, len(value)) + value
    if cls == 1 and kind == 16 and name.startswith("_acme-challenge."):
        path = STATE / (name + ".txt")
        if path.exists():
            value = path.read_bytes()
            if len(value) > 255:
                raise ValueError("oversize DNS challenge")
            value = bytes([len(value)]) + value
            answer = b"\xc0\x0c" + struct.pack("!HHIH", kind, 1, 1, len(value)) + value
    # No recursion, forwarding, host resolver fallback, or public traffic.
    flags = 0x8400 | (0 if name.endswith(".test") else 3)
    return packet[:2] + struct.pack("!HHHHH", flags, 1, bool(answer), 0, 0) + question + answer


class UDP(socketserver.BaseRequestHandler):
    def handle(self):
        packet, sock = self.request
        response = dns_answer(packet)
        if response:
            sock.sendto(response, self.client_address)


class TCP(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            length = struct.unpack("!H", exact(self.request, 2))[0]
            response = dns_answer(exact(self.request, length))
            self.request.sendall(struct.pack("!H", len(response)) + response)
        except (EOFError, OSError):
            pass


def dns():
    tcp = socketserver.ThreadingTCPServer(("0.0.0.0", 53), TCP)
    threading.Thread(target=tcp.serve_forever, daemon=True).start()
    socketserver.ThreadingUDPServer(("0.0.0.0", 53), UDP).serve_forever()


class Entrance(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass  # Matrix/OpenID access tokens must never enter logs.

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def handle_request(self):
        host = self.headers.get("Host", "").split(":")[0]
        if self.server.server_port == 8448 and host == "friend.test" and self.path.startswith("/_matrix/federation/v1/openid/userinfo?"):
            (STATE / "paired-userinfo").touch()
        if self.path == "/.well-known/matrix/server":
            self.reply({"m.server": host + ":443"})
            return
        if self.path == "/.well-known/wonton":
            # The entrance publishes its own core's authenticated runtime endpoint.
            core = "friend" if host == "friend.test" else "owner"
            connection = http.client.HTTPConnection("core-" + core, 8080, timeout=5)
            token = (STATE / core / "core/owner-token").read_text().strip()
            connection.request("GET", "/_wonton/api/tunnel", headers={
                "Host": core + ".test:8080", "Authorization": "Bearer " + token})
            response = connection.getresponse()
            if response.status != 200:
                self.send_error(503)
                return
            info = json.loads(response.read())
            connection.close()
            self.reply({"endpoint_id": info["endpoint_id"], "relay_urls": ["https://relay.test/"]})
            return
        server = {"owner.test": "owner", "friend.test": "friend"}.get(host, "external")
        connection = http.client.HTTPConnection("tuwunel-" + server, 8008, timeout=10)
        length = int(self.headers.get("Content-Length", 0))
        if length > 65536:
            self.send_error(413)
            return
        body = self.rfile.read(length)
        headers = {key: value for key, value in self.headers.items()
                   if key.lower() not in ("host", "connection", "transfer-encoding")}
        connection.request(self.command, self.path, body, headers)
        response = connection.getresponse()
        payload = response.read(1024 * 1024 + 1)
        if len(payload) > 1024 * 1024:
            self.send_error(502)
            return
        self.send_response(response.status)
        for key, value in response.getheaders():
            if key.lower() not in ("content-length", "connection", "transfer-encoding"):
                self.send_header(key, value)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)
        connection.close()

    do_GET = handle_request
    do_POST = handle_request


def frontend(bad):
    if bad:
        cert, key = STATE / "bad-cert.crt", STATE / "bad-cert.key"
    else:
        cert = STATE / "certs/live/share-smoke/fullchain.pem"
        key = STATE / "certs/live/share-smoke/privkey.pem"
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    # 联邦口两个前门都开：坏证书那个也要真的走到 TLS 校验，而不是连接被拒。
    paired = ThreadingHTTPServer(("0.0.0.0", 8448), Entrance)
    paired.socket = context.wrap_socket(paired.socket, server_side=True)
    threading.Thread(target=paired.serve_forever, daemon=True).start()
    server = ThreadingHTTPServer(("0.0.0.0", 443), Entrance)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    server.serve_forever()


def root():
    context = ssl.create_default_context(cafile=str(STATE / "pebble.crt"))
    directory = http.client.HTTPSConnection("pebble.test", 14000, context=context, timeout=2)
    directory.request("GET", "/dir")
    response = directory.getresponse()
    if response.status != 200:
        raise RuntimeError("Pebble ACME directory unavailable")
    json.loads(response.read(65536))
    directory.close()
    connection = http.client.HTTPSConnection("pebble.test", 15000, context=context, timeout=2)
    connection.request("GET", "/roots/0")
    response = connection.getresponse()
    if response.status != 200:
        raise RuntimeError("Pebble root retrieval failed")
    pem = response.read(65536)
    ssl.PEM_cert_to_DER_cert(pem.decode())
    (STATE / "ca.pem").write_bytes(pem)
    connection.close()


def hook(clean):
    domain = os.environ["CERTBOT_DOMAIN"]
    if not domain.endswith(".test") or "/" in domain:
        raise ValueError("ACME domain outside isolated .test zone")
    path = STATE / ("_acme-challenge." + domain + ".txt")
    if clean:
        path.unlink(missing_ok=True)
    else:
        path.write_text(os.environ["CERTBOT_VALIDATION"])


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "dns":
        dns()
    elif mode == "root":
        root()
    elif mode in ("auth", "cleanup"):
        hook(mode == "cleanup")
    elif mode in ("frontend", "bad-cert"):
        frontend(mode == "bad-cert")
    else:
        raise ValueError("unknown fixture mode")
