#!/usr/bin/env python3
"""Drive an Android WebView through Chrome DevTools Protocol."""

from __future__ import annotations

import base64
import json
import os
import re
import socket
import struct
import sys
import time
from dataclasses import dataclass
from urllib.parse import urlsplit
from urllib.request import urlopen


class CdpError(RuntimeError):
    """The WebView did not provide the requested CDP response."""


class WebSocket:
    def __init__(self, host: str, port: int, path: str) -> None:
        self.socket = socket.create_connection((host, port), timeout=15)
        self.socket.settimeout(30)
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        ).encode()
        self.socket.sendall(request)
        response = self._read_until(b"\r\n\r\n")
        if b" 101 " not in response.split(b"\r\n", 1)[0]:
            raise CdpError(f"CDP WebSocket upgrade failed: {response[:200]!r}")

    def _read_until(self, marker: bytes) -> bytes:
        data = bytearray()
        while marker not in data:
            chunk = self.socket.recv(4096)
            if not chunk:
                raise CdpError("CDP socket closed during handshake")
            data.extend(chunk)
        return bytes(data)

    def send(self, value: str) -> None:
        payload = value.encode()
        mask = os.urandom(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        length = len(payload)
        if length < 126:
            header = bytes([0x81, 0x80 | length])
        elif length < 65536:
            header = bytes([0x81, 0x80 | 126]) + struct.pack(">H", length)
        else:
            header = bytes([0x81, 0x80 | 127]) + struct.pack(">Q", length)
        self.socket.sendall(header + mask + masked)

    def receive(self) -> str:
        while True:
            first, second = self._read_exact(2)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack(">H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._read_exact(8))[0]
            masked = second & 0x80
            mask = self._read_exact(4) if masked else b""
            payload = bytearray(self._read_exact(length))
            if masked:
                for index in range(length):
                    payload[index] ^= mask[index % 4]
            if opcode == 0x8:
                raise CdpError("CDP WebSocket closed")
            if opcode == 0x9:
                self._send_control(0xA, bytes(payload))
                continue
            if opcode == 0x1:
                return bytes(payload).decode()

    def _send_control(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.socket.sendall(bytes([0x80 | opcode, 0x80 | len(payload)]) + mask + masked)

    def _read_exact(self, length: int) -> bytes:
        data = bytearray()
        while len(data) < length:
            chunk = self.socket.recv(length - len(data))
            if not chunk:
                raise CdpError("CDP socket closed")
            data.extend(chunk)
        return bytes(data)

    def close(self) -> None:
        try:
            self.socket.close()
        except OSError:
            pass


@dataclass
class Cdp:
    socket: WebSocket
    next_id: int = 0

    def command(self, method: str, params: dict | None = None) -> dict:
        self.next_id += 1
        command_id = self.next_id
        self.socket.send(json.dumps({"id": command_id, "method": method, "params": params or {}}))
        while True:
            message = json.loads(self.socket.receive())
            if message.get("id") == command_id:
                if "error" in message:
                    raise CdpError(f"{method}: {message['error']}")
                return message

    def evaluate(self, expression: str):
        result = self.command(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": True},
        )
        payload = result.get("result", {})
        if "exceptionDetails" in payload:
            raise CdpError(f"JavaScript failed: {payload['exceptionDetails']}")
        remote = payload.get("result", {})
        if remote.get("subtype") == "error":
            raise CdpError(remote.get("description", "JavaScript error"))
        return remote.get("value")


def targets(port: int) -> list[dict]:
    with urlopen(f"http://127.0.0.1:{port}/json/list", timeout=10) as response:
        return json.load(response)


def connect(port: int) -> Cdp:
    deadline = time.monotonic() + 60
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            pages = [page for page in targets(port) if page.get("type") == "page"]
            if pages:
                endpoint = urlsplit(pages[0]["webSocketDebuggerUrl"])
                return Cdp(WebSocket(endpoint.hostname or "127.0.0.1", endpoint.port or port, endpoint.path))
        except (OSError, ValueError, KeyError) as error:
            last_error = error
        time.sleep(1)
    detail = f": {last_error}" if last_error else ""
    raise CdpError(f"no WebView page exposed on localhost:{port}{detail}")


def wait_text(cdp: Cdp, pattern: str, timeout: float = 60) -> None:
    wanted = re.compile(pattern, re.IGNORECASE)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        text = cdp.evaluate("document.body.innerText") or ""
        if wanted.search(text):
            return
        time.sleep(1)
    raise CdpError(f"timed out waiting for page text /{pattern}/")


def install_instrumentation(cdp: Cdp) -> None:
    cdp.evaluate(
        """(() => {
          const Original = window.RTCPeerConnection;
          if (!Original || Original.__wontonWrapped) return;
          window.__wontonPeerConnections = [];
          const Wrapped = new Proxy(Original, {
            construct(target, args, newTarget) {
              const pc = Reflect.construct(target, args, newTarget);
              window.__wontonPeerConnections.push(pc);
              return pc;
            },
          });
          Object.defineProperty(Wrapped, '__wontonWrapped', { value: true });
          window.RTCPeerConnection = Wrapped;
        })()"""
    )


def click_join(cdp: Cdp) -> None:
    cdp.evaluate(
        """(() => {
          const button = [...document.querySelectorAll('button,[role=button]')]
            .find(element => /join|enter|加入|参加/i.test(element.innerText || element.getAttribute('aria-label') || ''));
          if (!button) throw new Error('Element Call join button was not found');
          button.click();
        })()"""
    )


def click_hangup(cdp: Cdp) -> None:
    cdp.evaluate(
        """(() => {
          const button = [...document.querySelectorAll('button,[role=button]')]
            .find(element => /hang.?up|leave|end call|挂断|离开通话/i.test(
              element.innerText || element.getAttribute('aria-label') || ''));
          if (!button) throw new Error('Element Call hang-up button was not found');
          button.click();
        })()"""
    )


def stats(cdp: Cdp):
    return cdp.evaluate(
        """(async () => {
          const peers = window.__wontonPeerConnections || [];
          const inbound = [];
          const pairs = [];
          for (const pc of peers) {
            const report = await pc.getStats();
            for (const value of report.values()) {
              if (value.type === 'inbound-rtp' && !value.isRemote) {
                inbound.push({
                  kind: value.kind || value.mediaType,
                  bytesReceived: value.bytesReceived || 0,
                  packetsReceived: value.packetsReceived || 0,
                  packetsLost: value.packetsLost || 0,
                  timestamp: value.timestamp || 0,
                });
              }
              if (value.type === 'candidate-pair' && value.state === 'succeeded' &&
                  (value.selected || value.nominated)) {
                pairs.push({
                  protocol: value.protocol,
                  localCandidateId: value.localCandidateId,
                  remoteCandidateId: value.remoteCandidateId,
                  state: value.state,
                });
              }
            }
            for (const value of report.values()) {
              if (value.type !== 'remote-candidate') continue;
              for (const pair of pairs) {
                if (pair.remoteCandidateId === value.id) {
                  pair.remote = { address: value.address, port: value.port,
                    protocol: value.protocol, type: value.candidateType };
                }
              }
            }
          }
          return { inbound, pairs, url: location.href,
            text: document.body.innerText.slice(0, 2000) };
        })()"""
    )


def action(cdp: Cdp, name: str, seconds: int) -> None:
    if name in {"join", "stats"}:
        install_instrumentation(cdp)
    if name == "join":
        click_join(cdp)
        wait_text(cdp, r"connected|joined|已加入|已连接", 60)
    elif name == "hangup":
        click_hangup(cdp)
        return
    elif name == "wait-left":
        wait_text(cdp, r"left|disconnected|离开|已退出|不在通话", seconds)
        return
    elif name == "relay-hint":
        text = cdp.evaluate("document.body.innerText") or ""
        print(json.dumps({"hint": bool(re.search(r"relay|中继|转发", text, re.IGNORECASE)), "text": text[:2000]}, ensure_ascii=False))
        return
    elif name != "stats":
        raise CdpError(f"unknown action: {name}")
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        print(json.dumps(stats(cdp), ensure_ascii=False), flush=True)
        time.sleep(3)


def main() -> int:
    if len(sys.argv) < 2:
        print(f"usage: {sys.argv[0]} <port> [join|stats|hangup|wait-left|relay-hint] [seconds]", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    name = sys.argv[2] if len(sys.argv) > 2 else "stats"
    seconds = int(sys.argv[3]) if len(sys.argv) > 3 else 30
    cdp = connect(port)
    try:
        cdp.command("Runtime.enable")
        action(cdp, name, seconds)
    finally:
        cdp.socket.close()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CdpError, OSError, ValueError) as error:
        print(f"call-cdp: {error}", file=sys.stderr)
        raise SystemExit(1)
