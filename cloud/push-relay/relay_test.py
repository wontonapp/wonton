#!/usr/bin/env python3
"""relay.py 的行为测试：VAPID 校验、限速、Web Push 到 APNs 的翻译和响应码。

跑法：python3 -m unittest relay_test -v（需要 cryptography）。
"""

import base64
import json
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from tempfile import TemporaryDirectory

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec, utils
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

import relay

ORIGIN = "https://push.example"
TOKEN = "ab" * 32
FCM = "dQw4w9WgXc:APA91bHun4MxP5egoKMwt2KZFBaFUH-1KYqYpe2FctCwFeaQcUp7i3Lp6Z3y0Yz8y0V"


def b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def key_pair() -> ec.EllipticCurvePrivateKey:
    return ec.generate_private_key(ec.SECP256R1())


def public_key(key: ec.EllipticCurvePrivateKey) -> str:
    return b64(key.public_key().public_bytes(Encoding.X962, PublicFormat.UncompressedPoint))


def jwt(key: ec.EllipticCurvePrivateKey, aud: str = ORIGIN, exp: int = 3600, **claims) -> str:
    head = b64(json.dumps({"typ": "JWT", "alg": "ES256"}).encode())
    body = {
        "aud": aud,
        "exp": int(time.time()) + exp,
        "sub": "mailto:ops@example",
        **claims,
    }
    payload = b64(json.dumps(body).encode())
    der = key.sign(f"{head}.{payload}".encode(), ec.ECDSA(hashes.SHA256()))
    r, s = utils.decode_dss_signature(der)
    return f"{head}.{payload}.{b64(r.to_bytes(32, 'big') + s.to_bytes(32, 'big'))}"


def vapid(key: ec.EllipticCurvePrivateKey, **kwargs) -> str:
    return f"vapid t={jwt(key, **kwargs)}, k={public_key(key)}"




class VerifierTest(unittest.TestCase):
    def setUp(self):
        self.key = key_pair()
        raw = self.key.public_key().public_bytes(Encoding.X962, PublicFormat.UncompressedPoint)
        self.server = relay.Server(
            name="home",
            public_key=self.key.public_key(),
            raw_key=raw,
            thumbprint=relay.thumbprint(raw),
            limit=relay.Bucket(1, 1),
        )
        self.verifier = relay.Verifier(ORIGIN, {self.server.thumbprint: self.server})

    def test_accepts_a_signed_token(self):
        self.assertIs(self.verifier.verify(vapid(self.key)), self.server)

    def test_accepts_the_same_origin_with_a_port(self):
        self.assertIs(self.verifier.verify(vapid(self.key, aud="https://push.example:8443")), self.server)

    def test_rejects_other_audience(self):
        with self.assertRaises(relay.VapidError):
            self.verifier.verify(vapid(self.key, aud="https://other.example"))

    def test_rejects_expired_token(self):
        with self.assertRaises(relay.VapidError):
            self.verifier.verify(vapid(self.key, exp=-120))

    def test_tolerates_expiry_within_a_minute(self):
        self.assertIs(self.verifier.verify(vapid(self.key, exp=-30)), self.server)

    def test_rejects_future_nbf(self):
        with self.assertRaises(relay.VapidError):
            self.verifier.verify(vapid(self.key, nbf=int(time.time()) + 120))

    def test_rejects_unknown_key(self):
        with self.assertRaises(relay.VapidError):
            self.verifier.verify(vapid(key_pair()))

    def test_rejects_tampered_payload(self):
        token = vapid(self.key).split("t=")[1].split(", k=")[0]
        head, payload, signature = token.split(".")
        claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
        claims["aud"] = "https://evil.example"
        forged = b64(json.dumps(claims).encode())
        with self.assertRaises(relay.VapidError):
            self.verifier.verify(f"vapid t={head}.{forged}.{signature}, k={public_key(self.key)}")

    def test_rejects_wrong_scheme(self):
        with self.assertRaises(relay.VapidError):
            self.verifier.verify("bearer abc")

    def test_rejects_missing_parts(self):
        with self.assertRaises(relay.VapidError):
            self.verifier.verify("vapid t=abc")
        with self.assertRaises(relay.VapidError):
            self.verifier.verify(f"vapid k={public_key(self.key)}")


class BucketTest(unittest.TestCase):
    def test_allows_the_burst_then_waits(self):
        bucket = relay.Bucket(1.0, 2)
        self.assertEqual(bucket.take(), 0)
        self.assertEqual(bucket.take(), 0)
        self.assertGreater(bucket.take(), 0.5)


class FilesTest(unittest.TestCase):
    def setUp(self):
        self.dir = TemporaryDirectory()
        self.path = Path(self.dir.name)

    def tearDown(self):
        self.dir.cleanup()

    def write(self, name: str, data: dict) -> Path:
        path = self.path / name
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def test_loads_a_registry_entry(self):
        key = key_pair()
        self.write("home.json", {"name": "home", "public_key": public_key(key), "rate_per_minute": 60, "burst": 5})
        servers = relay.load_servers(self.path)
        self.assertEqual([s.name for s in servers.values()], ["home"])

    def test_rejects_duplicate_keys(self):
        key = key_pair()
        entry = {"name": "a", "public_key": public_key(key), "rate_per_minute": 60, "burst": 5}
        self.write("a.json", entry)
        self.write("b.json", {**entry, "name": "b"})
        with self.assertRaises(relay.ConfigError):
            relay.load_servers(self.path)

    def test_rejects_point_off_the_curve(self):
        self.write("bad.json", {"name": "x", "public_key": b64(b"\x04" + b"\x01" * 64), "rate_per_minute": 60, "burst": 5})
        with self.assertRaises(relay.ConfigError):
            relay.load_servers(self.path)

    def test_an_empty_registry_loads(self):
        self.assertEqual(relay.load_servers(self.path), {})


class FakeGorush(BaseHTTPRequestHandler):
    answer = {"success": "ok", "logs": []}
    status = 200
    seen: list[dict] = []

    def log_message(self, *_args):
        pass

    def do_POST(self):  # noqa: N802
        length = int(self.headers["Content-Length"])
        FakeGorush.seen.append(json.loads(self.rfile.read(length)))
        body = json.dumps(FakeGorush.answer).encode()
        self.send_response(FakeGorush.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802
        self.send_response(FakeGorush.status)
        self.send_header("Content-Length", "0")
        self.end_headers()


class HandlerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.gorush = HTTPServer(("127.0.0.1", 0), FakeGorush)
        threading.Thread(target=cls.gorush.serve_forever, daemon=True).start()
        cls.gorush_url = f"http://127.0.0.1:{cls.gorush.server_port}/api/push"

    @classmethod
    def tearDownClass(cls):
        cls.gorush.shutdown()
        cls.gorush.server_close()

    def setUp(self):
        FakeGorush.answer = {"success": "ok", "logs": []}
        FakeGorush.status = 200
        FakeGorush.seen = []
        key = key_pair()
        raw = key.public_key().public_bytes(Encoding.X962, PublicFormat.UncompressedPoint)
        self.key = key
        self.topic = "app.wonton.ios"

        handler = type("TestHandler", (relay.Handler,), {})
        handler.config = relay.Config(
            listen=("127.0.0.1", 0),
            gorush_url=self.gorush_url,
            gorush_health_url=self.gorush_url.replace("/api/push", "/healthz"),
            origin=ORIGIN,
            apns_topic=self.topic,
            alert="新消息",
            max_body=relay.MAX_BODY,
            timeout=5,
            servers_dir=Path("/nonexistent"),
            global_limit=relay.GlobalLimit(600, 200),
        )
        self.server = relay.Server(
            name="home",
            public_key=key.public_key(),
            raw_key=raw,
            thumbprint=relay.thumbprint(raw),
            limit=relay.Bucket(600 / 60.0, 60),
        )
        handler.verifier = relay.Verifier(ORIGIN, {self.server.thumbprint: self.server})
        handler.gorush = relay.Gorush(
            self.gorush_url, self.gorush_url.replace("/api/push", "/healthz"), "新消息", self.topic, 5
        )
        handler.global_bucket = relay.Bucket(600 / 60.0, 200)
        self.httpd = HTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.httpd.server_port}"

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.server_close()

    def post(self, path: str = f"/push/{TOKEN}", body: bytes = b"cipher", headers: dict | None = None):
        request = urllib.request.Request(self.base + path, data=body, method="POST")
        request.add_header("Content-Encoding", "aes128gcm")
        request.add_header("Authorization", vapid(self.key))
        for name, value in (headers or {}).items():
            request.add_header(name, value)
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                return response.status, response.headers
        except urllib.error.HTTPError as error:
            return error.code, error.headers

    def fcm_post(self, path: str = f"/push/{FCM}", body: bytes = b"cipher"):
        request = urllib.request.Request(self.base + path, data=body, method="POST")
        request.add_header("Content-Encoding", "aes128gcm")
        request.add_header("Authorization", vapid(self.key))
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                return response.status, response.headers
        except urllib.error.HTTPError as error:
            return error.code, error.headers

    def test_maps_web_push_to_fcm_data(self):
        status, _ = self.fcm_post()
        self.assertEqual(status, 201)
        sent = FakeGorush.seen[0]["notifications"][0]
        self.assertEqual(sent["platform"], 2)
        self.assertEqual(sent["tokens"], [FCM])
        self.assertEqual(base64.urlsafe_b64decode(sent["data"]["wonton"] + "=="), b"cipher")
        self.assertNotIn("message", sent, "data 消息不要系统弹的那条通知")
        self.assertNotIn("topic", sent)

    def test_opens_an_fcm_push_endpoint(self):
        request = urllib.request.Request(
            self.base + "/v1/fcm",
            data=json.dumps({"token": FCM}).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=5) as response:
            self.assertEqual(response.status, 200)
            answer = json.loads(response.read())
        self.assertEqual(answer["endpoint"], f"{ORIGIN}/push/{FCM}")
        self.assertEqual(FakeGorush.seen, [])

    def test_bad_fcm_token_is_400(self):
        request = urllib.request.Request(
            self.base + "/v1/fcm",
            data=json.dumps({"token": "short"}).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(request, timeout=5)
        self.assertEqual(caught.exception.code, 400)

    def test_fcm_unregistered_is_410(self):
        FakeGorush.answer = {"logs": [{"error": "Unregistered"}]}
        self.assertEqual(self.fcm_post()[0], 410)

    def test_maps_web_push_to_apns(self):
        status, _ = self.post(headers={"Urgency": "high", "TTL": "60", "Topic": "room-1"})
        self.assertEqual(status, 201)
        sent = FakeGorush.seen[0]["notifications"][0]
        self.assertEqual(sent["tokens"], [TOKEN])
        self.assertEqual(sent["push_type"], "alert")
        self.assertEqual(sent["priority"], "high")
        self.assertEqual(sent["collapse_id"], "room-1")
        self.assertEqual(sent["topic"], self.topic)
        self.assertEqual(base64.urlsafe_b64decode(sent["data"]["wonton"] + "=="), b"cipher")
        self.assertLessEqual(sent["expiration"], int(time.time()) + 61)

    def test_defaults_are_normal_priority_and_a_day(self):
        self.assertEqual(self.post()[0], 201)
        sent = FakeGorush.seen[0]["notifications"][0]
        self.assertEqual(sent["priority"], "normal")
        self.assertLessEqual(sent["expiration"], int(time.time()) + 86401)

    def test_ttl_zero_expires_immediately(self):
        self.assertEqual(self.post(headers={"TTL": "0"})[0], 201)
        self.assertEqual(FakeGorush.seen[0]["notifications"][0]["expiration"], 0)

    def test_bad_token_is_410(self):
        FakeGorush.answer = {"logs": [{"error": "BadDeviceToken"}]}
        self.assertEqual(self.post()[0], 410)

    def test_unregistered_is_410(self):
        FakeGorush.answer = {"logs": [{"error": "Unregistered"}]}
        self.assertEqual(self.post()[0], 410)

    def test_unavailable_is_503_with_retry_after(self):
        FakeGorush.answer = {"logs": [{"error": "Unavailable"}]}
        status, headers = self.post()
        self.assertEqual(status, 503)
        self.assertEqual(headers.get("Retry-After"), "1")
        # 中继自己写回的头要从 _classify 的 Reply 里透传出来。
        FakeGorush.answer = {"logs": [{"error": "TooManyRequests"}]}
        self.assertEqual(self.fcm_post()[0], 503)

    def test_message_too_big_is_413(self):
        FakeGorush.answer = {"logs": [{"error": "MessageTooBig"}]}
        self.assertEqual(self.fcm_post()[0], 413)

    def test_provider_error_is_502(self):
        FakeGorush.answer = {"logs": [{"error": "InvalidProviderToken"}]}
        self.assertEqual(self.post()[0], 502)

    def test_gorush_failure_is_502(self):
        FakeGorush.status = 500
        self.assertEqual(self.post()[0], 502)

    def test_missing_authorization_is_401(self):
        request = urllib.request.Request(self.base + f"/push/{TOKEN}", data=b"x", method="POST")
        request.add_header("Content-Encoding", "aes128gcm")
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(request, timeout=5)
        self.assertEqual(caught.exception.code, 401)

    def test_wrong_encoding_is_415(self):
        request = urllib.request.Request(self.base + f"/push/{TOKEN}", data=b"x", method="POST")
        request.add_header("Authorization", vapid(self.key))
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(request, timeout=5)
        self.assertEqual(caught.exception.code, 415)

    def test_oversized_body_is_413(self):
        self.assertEqual(self.post(body=b"x" * (relay.MAX_BODY + 1))[0], 413)
        self.assertEqual(FakeGorush.seen, [])

    def test_short_token_is_400(self):
        self.assertEqual(self.post(path="/push/abc")[0], 400)

    def test_unknown_path_is_404(self):
        self.assertEqual(self.post(path="/other")[0], 404)

    def test_second_request_over_burst_is_429(self):
        self.server.limit.capacity = 1
        self.server.limit.tokens = 1
        self.server.limit.rate_per_second = 0.001
        self.assertEqual(self.post()[0], 201)
        request = urllib.request.Request(self.base + f"/push/{TOKEN}", data=b"x", method="POST",
                                         headers={"Content-Encoding": "aes128gcm", "Authorization": vapid(self.key)})
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(request, timeout=5)
        self.assertEqual(caught.exception.code, 429)
        self.assertGreaterEqual(int(caught.exception.headers["Retry-After"]), 1)

    def test_health_reports_gorush(self):
        with urllib.request.urlopen(self.base + "/healthz", timeout=5) as response:
            self.assertEqual(response.status, 200)
        FakeGorush.status = 500
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(self.base + "/healthz", timeout=5)
        self.assertEqual(caught.exception.code, 503)


if __name__ == "__main__":
    unittest.main()
