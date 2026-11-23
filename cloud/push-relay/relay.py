#!/usr/bin/env python3
"""推送中继的包装器：Web Push（RFC 8030、RFC 8292）进，gorush（APNs）出。

家里服务器按 Web Push 发来的正文已经用设备的公钥加密过（RFC 8291），这里不解析、不落正文，只把它转成
APNs 的 `wonton` 字段交给 gorush。鉴权靠 VAPID：JWT 的 ES256 签名、`aud` 和 `exp` 都要过，签名用的公钥必须
在 `servers.d` 里登记过；限速按登记的那把公钥算，登记之外的一律 401。

只绑 127.0.0.1，公网入口是 nginx。
"""

from __future__ import annotations

import base64
import binascii
import json
import logging
import math
import signal
import sys
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature

LOG = logging.getLogger("relay")

# APNs 一条通知的上限是 4096 字节，`aps`、JSON 外壳和 base64url 的膨胀（4/3）要从里面扣掉。
# 本服务给每条请求加 ~200 字节（aps、expiration、topic、字段名），密文 base64 之后每 3 字节变 4 字节，
# 2816 字节的密文最多变成 ~3755 字节，加上外壳仍在 4096 以内。
MAX_BODY = 2816

# VAPID JWT 的 exp 最多 24 小时（RFC 8292 §2），时钟偏差留一分钟。
CLOCK_SKEW = 60

# 设备令牌是 32 字节的十六进制。
TOKEN = set("0123456789abcdef")


class ConfigError(Exception):
    pass


class VapidError(Exception):
    pass


class RequestError(Exception):
    """请求本身有问题，带着要回给客户端的响应码。"""

    def __init__(self, status: int, message: str, headers: dict[str, str] | None = None):
        super().__init__(message)
        self.status = status
        self.headers = headers or {}


@dataclass(frozen=True)
class GlobalLimit:
    rate_per_minute: float
    burst: int


@dataclass(frozen=True)
class Config:
    listen: tuple[str, int]
    gorush_url: str
    gorush_health_url: str
    origin: str
    apns_topic: str
    alert: str
    max_body: int
    timeout: float
    servers_dir: Path
    global_limit: GlobalLimit


@dataclass
class Bucket:
    """令牌桶。`take` 返回 0 表示放行，否则是还要等多少秒才有下一个令牌。"""

    rate_per_second: float
    capacity: float
    tokens: float = field(init=False)
    updated: float = field(init=False)

    def __post_init__(self) -> None:
        self.tokens = self.capacity
        self.updated = time.monotonic()

    def take(self) -> float:
        now = time.monotonic()
        self.tokens = min(self.capacity, self.tokens + (now - self.updated) * self.rate_per_second)
        self.updated = now
        if self.tokens >= 1:
            self.tokens -= 1
            return 0.0
        return (1 - self.tokens) / self.rate_per_second


@dataclass(frozen=True)
class Server:
    name: str
    public_key: ec.EllipticCurvePublicKey
    raw_key: bytes
    thumbprint: str
    limit: Bucket


def _read_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ConfigError(f"读不了 {path}：{error}") from error
    except json.JSONDecodeError as error:
        raise ConfigError(f"{path} 不是合法 JSON：{error}") from error


def _required(data: dict, key: str, where: str) -> object:
    if key not in data:
        raise ConfigError(f"{where} 缺 {key}")
    return data[key]


def load_config(path: Path) -> Config:
    data = _read_json(path)
    where = str(path)
    host, _, port = str(_required(data, "listen", where)).rpartition(":")
    if not host or not port.isdigit():
        raise ConfigError(f"{where} 的 listen 要写成 主机:端口")
    origin = str(_required(data, "origin", where)).rstrip("/")
    # 公网入口必须 https；只有本机回环允许 http，方便在主机上直接验。
    if not origin.startswith("https://") and _origin(origin) not in (
        "http://127.0.0.1",
        "http://localhost",
        "http://[::1]",
    ):
        raise ConfigError(f"{where} 的 origin 要是 https 地址（本机回环可以 http）")
    servers_dir = Path(str(_required(data, "servers_dir", where)))
    if not servers_dir.is_absolute():
        raise ConfigError(f"{where} 的 servers_dir 要是绝对路径")
    limit = data["global_limit"]
    return Config(
        listen=(host, int(port)),
        gorush_url=str(_required(data, "gorush_url", where)),
        gorush_health_url=str(_required(data, "gorush_health_url", where)),
        origin=origin,
        apns_topic=str(_required(data, "apns_topic", where)),
        alert=str(_required(data, "alert", where)),
        max_body=int(data.get("max_body", MAX_BODY)),
        timeout=float(data.get("timeout_seconds", 10)),
        servers_dir=servers_dir,
        global_limit=GlobalLimit(float(limit["rate_per_minute"]), int(limit["burst"])),
    )


def b64url_decode(text: str) -> bytes:
    try:
        return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))
    except (binascii.Error, ValueError) as error:
        raise VapidError("base64url 解不开") from error


def thumbprint(raw_key: bytes) -> str:
    digest = hashes.Hash(hashes.SHA256())
    digest.update(raw_key)
    return base64.urlsafe_b64encode(digest.finalize()).decode().rstrip("=")


def load_servers(servers_dir: Path) -> dict[str, Server]:
    servers: dict[str, Server] = {}
    for path in sorted(servers_dir.glob("*.json")):
        data = _read_json(path)
        where = str(path)
        raw = b64url_decode(str(_required(data, "public_key", where)))
        if len(raw) != 65 or raw[0] != 0x04:
            raise ConfigError(f"{where} 的 public_key 不是未压缩的 P-256 点")
        try:
            public_key = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), raw)
        except ValueError as error:
            raise ConfigError(f"{where} 的 public_key 不在 P-256 曲线上：{error}") from error
        key = thumbprint(raw)
        if key in servers:
            raise ConfigError(f"{where} 的公钥和 {servers[key].name} 重复")
        rate = float(_required(data, "rate_per_minute", where))
        burst = int(_required(data, "burst", where))
        if rate <= 0 or burst < 1:
            raise ConfigError(f"{where} 的 rate_per_minute 要大于 0，burst 至少为 1")
        servers[key] = Server(
            name=str(_required(data, "name", where)),
            public_key=public_key,
            raw_key=raw,
            thumbprint=key,
            limit=Bucket(rate / 60.0, burst),
        )
    return servers

def _origin(url: str) -> str:
    """去掉路径和端口，只留 `scheme://host`。"""
    scheme, sep, rest = url.partition("://")
    return f"{scheme}://{rest.split('/')[0].split(':')[0]}" if sep else url


# RFC 8292 §3：`Authorization: vapid t=<JWT>, k=<公钥>`。参数顺序不固定，值里不含逗号、空格。
class Verifier:
    """校验 VAPID 的 JWT，返回签名的那台服务器。"""

    def __init__(self, origin: str, servers: dict[str, Server]):
        self.origin = origin
        self.servers = servers

    def verify(self, header: str, now: float | None = None) -> Server:
        scheme, _, rest = header.partition(" ")
        if scheme.lower() != "vapid":
            raise VapidError("不是 vapid 方案")
        parts: dict[str, str] = {}
        for item in rest.split(","):
            name, sep, value = item.strip().partition("=")
            if not sep or not name or not value:
                raise VapidError("vapid 参数不完整")
            parts[name] = value
        raw_key = b64url_decode(parts.get("k", ""))
        if len(raw_key) != 65 or raw_key[0] != 0x04:
            raise VapidError("vapid 的 k 不是未压缩的 P-256 点")
        server = self.servers.get(thumbprint(raw_key))
        if server is None:
            raise VapidError("这把公钥没有登记")
        token = parts.get("t", "").split(".")
        if len(token) != 3:
            raise VapidError("vapid 的 t 不是 JWT")
        header_json = json.loads(b64url_decode(token[0]))
        if header_json.get("alg") != "ES256":
            raise VapidError("JWT 不是 ES256")
        signature = b64url_decode(token[2])
        if len(signature) != 64:
            raise VapidError("JWT 签名长度不对")
        try:
            server.public_key.verify(
                encode_dss_signature(
                    int.from_bytes(signature[:32], "big"), int.from_bytes(signature[32:], "big")
                ),
                f"{token[0]}.{token[1]}".encode(),
                ec.ECDSA(hashes.SHA256()),
            )
        except InvalidSignature as error:
            raise VapidError("JWT 签名不对") from error
        claims = json.loads(b64url_decode(token[1]))
        # 签发的库（web-push-native）把 aud 拍到不带端口；RFC 8292 的 aud 是源，带端口也认。
        if _origin(str(claims.get("aud"))) != _origin(self.origin):
            raise VapidError(f"JWT 的 aud 不是 {self.origin}")
        if not claims.get("sub"):
            raise VapidError("JWT 缺 sub")
        clock = time.time() if now is None else now
        expires = claims.get("exp")
        if not isinstance(expires, int) or expires < clock - CLOCK_SKEW:
            raise VapidError("JWT 过期了")
        not_before = claims.get("nbf")
        if isinstance(not_before, int) and not_before > clock + CLOCK_SKEW:
            raise VapidError("JWT 还没生效")
        return server


@dataclass
class Reply:
    status: int
    body: bytes


class Gorush:
    """gorush 的 `/api/push`。只走本机回环，不吃环境里的代理。"""

    def __init__(self, url: str, health_url: str, alert: str, topic: str, timeout: float):
        self.url = url
        self.health_url = health_url
        self.alert = alert
        self.topic = topic
        self.timeout = timeout
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def healthy(self) -> bool:
        try:
            with self.opener.open(self.health_url, timeout=2) as response:
                return response.status == HTTPStatus.OK
        except (urllib.error.URLError, OSError):
            return False

    def push(
        self, token: str, payload: bytes, priority: str, expiration: int, collapse: str
    ) -> Reply:
        body = {
            "notifications": [
                {
                    "tokens": [token],
                    "platform": 1,
                    "message": self.alert,  # 通知扩展解密前显示的兜底正文
                    "mutable_content": True,
                    "content_available": True,
                    "push_type": "alert",
                    "priority": priority,
                    "expiration": expiration,
                    "topic": self.topic,
                    "data": {"wonton": base64.urlsafe_b64encode(payload).decode().rstrip("=")},
                }
            ]
        }
        if collapse:
            body["notifications"][0]["collapse_id"] = collapse
        request = urllib.request.Request(
            self.url,
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                return self._classify(json.loads(response.read() or b"{}"))
        except urllib.error.HTTPError as error:
            LOG.warning("gorush 回了 %s：%s", error.code, error.read()[:200])
            return Reply(HTTPStatus.BAD_GATEWAY, b'{"error":"push service unavailable"}')
        except (urllib.error.URLError, OSError, json.JSONDecodeError) as error:
            LOG.warning("gorush 不可用：%s", error)
            return Reply(HTTPStatus.BAD_GATEWAY, b'{"error":"push service unavailable"}')

    @staticmethod
    def _classify(answer: dict) -> Reply:
        logs = answer.get("logs") or []
        if not logs:
            return Reply(HTTPStatus.CREATED, b"")
        reason = str(logs[0].get("error") or "unknown")
        if reason in ("BadDeviceToken", "Unregistered", "DeviceTokenNotForTopic"):
            # 设备换了令牌或卸载了 App：等价于 Web Push 的 410，核心据此删掉登记。
            return Reply(HTTPStatus.GONE, json.dumps({"error": reason}, ensure_ascii=False).encode())
        if reason == "TooManyRequests":
            return Reply(
                HTTPStatus.SERVICE_UNAVAILABLE,
                json.dumps({"error": reason}, ensure_ascii=False).encode(),
                {"Retry-After": "1"},
            )
        LOG.warning("APNs 拒绝：%s", reason)
        return Reply(HTTPStatus.BAD_GATEWAY, json.dumps({"error": reason}, ensure_ascii=False).encode())


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "wonton-push-relay"
    # 由 `serve` 填。
    config: Config
    verifier: Verifier
    gorush: Gorush
    global_bucket: Bucket

    def log_message(self, fmt: str, *args) -> None:  # noqa: A003 - 标准库的签名
        LOG.debug("%s - %s", self.address_string(), fmt % args)

    def do_GET(self) -> None:  # noqa: N802 - 标准库的签名
        if self.path != "/healthz":
            self._reply(Reply(HTTPStatus.NOT_FOUND, b'{"error":"not found"}'))
            return
        if self.gorush.healthy():
            self._reply(Reply(HTTPStatus.OK, b'{"ok":true}'))
        else:
            self._reply(Reply(HTTPStatus.SERVICE_UNAVAILABLE, b'{"error":"gorush down"}'))

    def do_POST(self) -> None:  # noqa: N802 - 标准库的签名
        started = time.monotonic()
        server = None
        token = ""
        size = 0
        try:
            # 先读完正文：404、401、413 都在读完正文之后才判，不读就会被客户端写成 RST。
            payload = self._body()
            size = len(payload)
            token = self._token()
            server = self._authorize()
            self._limit(server)
            reply = self._forward(token, payload)
        except RequestError as error:
            self._reply(Reply(error.status, json.dumps({"error": str(error)}, ensure_ascii=False).encode()), error.headers)
            reply = None
        except OSError:
            # 读请求时对端断了，浏览器/核心自己会重试。
            return
        if reply is not None:
            self._reply(reply)
        LOG.info(
            "POST push/%s %s %dB %.0fms%s",
            token[:8] or "-",
            reply.status if reply else "-",
            size,
            (time.monotonic() - started) * 1000,
            f" server={server.name}" if server else "",
        )

    def _token(self) -> str:
        if not self.path.startswith("/push/"):
            raise RequestError(HTTPStatus.NOT_FOUND, "not found")
        token = self.path[len("/push/") :].partition("?")[0].lower()
        if len(token) != 64 or not set(token) <= TOKEN:
            raise RequestError(HTTPStatus.BAD_REQUEST, "device token is not 64 hex chars")
        return token

    def _body(self) -> bytes:
        encoding = self.headers.get("Content-Encoding", "")
        if encoding != "aes128gcm":
            raise RequestError(HTTPStatus.UNSUPPORTED_MEDIA_TYPE, "Content-Encoding must be aes128gcm")
        length = self.headers.get("Content-Length")
        if length is None:
            raise RequestError(HTTPStatus.LENGTH_REQUIRED, "Content-Length required")
        if not length.isdigit():
            raise RequestError(HTTPStatus.BAD_REQUEST, "bad Content-Length")
        size = int(length)
        if size == 0:
            raise RequestError(HTTPStatus.BAD_REQUEST, "empty body")
        # 先把正文读掉再判大小：不读完就回 413，客户端还在写，连接会被 RST 掉。
        payload = self.rfile.read(min(size, 65536))
        if size > 65536:
            self.close_connection = True
        if size > self.config.max_body:
            raise RequestError(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE, f"body over {self.config.max_body} bytes"
            )
        if len(payload) != size:
            raise RequestError(HTTPStatus.BAD_REQUEST, "short body")
        return payload

    def _authorize(self) -> Server:
        header = self.headers.get("Authorization")
        if not header:
            raise RequestError(HTTPStatus.UNAUTHORIZED, "missing VAPID authorization")
        if len(header) > 4096:
            raise RequestError(HTTPStatus.UNAUTHORIZED, "VAPID: authorization too long")
        try:
            return self.verifier.verify(header)
        except (VapidError, ValueError) as error:
            raise RequestError(HTTPStatus.UNAUTHORIZED, f"VAPID: {error}") from error

    def _limit(self, server: Server) -> None:
        waits = [server.limit.take(), self.global_bucket.take()]
        wait = max(waits)
        if wait > 0:
            raise RequestError(
                HTTPStatus.TOO_MANY_REQUESTS,
                "rate limited",
                {"Retry-After": str(max(1, math.ceil(wait)))},
            )

    def _forward(self, token: str, payload: bytes) -> Reply:
        urgency = (self.headers.get("Urgency") or "normal").lower()
        priority = "high" if urgency == "high" else "normal"
        ttl = self._ttl()
        expiration = 0 if ttl == 0 else int(time.time()) + ttl
        collapse = (self.headers.get("Topic") or "")[:64]
        return self.gorush.push(token, payload, priority, expiration, collapse)

    def _ttl(self) -> int:
        raw = self.headers.get("TTL")
        if raw is None:
            return 86400
        if not raw.isdigit():
            raise RequestError(HTTPStatus.BAD_REQUEST, "TTL must be seconds")
        return min(int(raw), 86400)

    def _reply(self, reply: Reply, headers: dict[str, str] | None = None) -> None:
        self.send_response(reply.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(reply.body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(reply.body)


class HttpServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    request_queue_size = 128


def serve(config_path: Path) -> int:
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s", stream=sys.stdout
    )
    try:
        config = load_config(config_path)
        servers = load_servers(config.servers_dir)
    except ConfigError as error:
        LOG.error("%s", error)
        return 1
    if servers:
        LOG.info("登记了 %d 台服务器：%s", len(servers), ", ".join(s.name for s in servers.values()))
    else:
        LOG.warning("还没有登记任何服务器，推送都会 401（server-add.sh）")

    handler = type("BoundHandler", (Handler,), {})
    handler.config = config
    handler.verifier = Verifier(config.origin, servers)
    handler.gorush = Gorush(
        config.gorush_url, config.gorush_health_url, config.alert, config.apns_topic, config.timeout
    )
    handler.global_bucket = Bucket(
        config.global_limit.rate_per_minute / 60.0, config.global_limit.burst
    )

    def reload() -> None:
        try:
            loaded = load_servers(config.servers_dir)
        except ConfigError as error:
            LOG.error("重新读登记表失败，沿用旧的：%s", error)
            return
        handler.verifier.servers = loaded
        LOG.info("重新读了登记表：%d 台", len(loaded))

    signal.signal(signal.SIGHUP, lambda *_: threading.Thread(target=reload, daemon=True).start())

    httpd = HttpServer(config.listen, handler)
    signal.signal(signal.SIGTERM, lambda *_: threading.Thread(target=httpd.shutdown, daemon=True).start())
    signal.signal(signal.SIGINT, lambda *_: threading.Thread(target=httpd.shutdown, daemon=True).start())
    LOG.info("监听 %s:%d", *config.listen)
    httpd.serve_forever()
    httpd.server_close()
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(f"用法：{sys.argv[0]} <配置文件>", file=sys.stderr)
        sys.exit(2)
    sys.exit(serve(Path(sys.argv[1])))
