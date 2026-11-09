#!/usr/bin/env python3
"""Run ADR-0051 acceptance with isolated local DNS/CA and two installed homes."""
import hashlib
import hmac
import http.client
import ipaddress
import json
import os
from pathlib import Path
import shutil
import socket
import sys
import ssl
import subprocess
import tempfile
import tomllib
from urllib.parse import quote

from smoke_support import HERE, REPO, SLOT, RUNTIME_IMAGE, Home, curl, free_port, run, wait_for


def account(home, username):
    nonce = home.matrix("/_synapse/admin/v1/register")["nonce"]
    password = os.urandom(32).hex()
    secret = (home.install / "secrets/matrix-registration-secret").read_text().strip().encode()
    mac = hmac.new(secret, "\0".join([nonce, username, password, "notadmin"]).encode(), hashlib.sha1).hexdigest()
    return home.matrix("/_synapse/admin/v1/register", {
        "nonce": nonce, "username": username, "password": password, "mac": mac,
    })


def chat(a, b, alice, bob, label):
    room = a.matrix("/_matrix/client/v3/createRoom", {"preset": "private_chat"}, alice["access_token"])["room_id"]
    path = "/_matrix/client/v3/rooms/" + quote(room, safe="")
    a.matrix(path + "/invite", {"user_id": bob["user_id"]}, alice["access_token"])
    b.matrix(path + "/join", {}, bob["access_token"])
    body = label + ":" + os.urandom(8).hex()
    a.matrix(path + "/send/m.room.message/" + os.urandom(8).hex(),
             {"msgtype": "m.text", "body": body}, alice["access_token"], "PUT")
    def delivered():
        reply = b.matrix(path + "/messages?dir=b&limit=20", token=bob["access_token"])
        return any(event.get("content", {}).get("body") == body for event in reply["chunk"])
    wait_for(label, delivered)
    print("PASS " + label, flush=True)


def entry(home, path):
    try:
        status, body = curl(home.cloud["network"], "https://" + home.name + path, ca=home.cloud["ca"],
                            dns=home.cloud["resolver"])
    except ConnectionError:
        logs = home.compose("logs", "--no-color", "--tail", "120", check=False)
        print(logs.stdout + logs.stderr, flush=True)
        relay = run("bash", HERE / "dev.sh", "compose", "logs", "--no-color", "--tail", "160", "frps", "home", check=False)
        print(relay.stdout + relay.stderr, flush=True)
        raise
    assert status == 200, f"{path}: {status} {body}"
    return json.loads(body)


def _entry_ready(home):
    try:
        status, body = curl(home.cloud["network"], "https://" + home.name + "/.well-known/matrix/server",
                            ca=home.cloud["ca"], dns=home.cloud["resolver"])
    except ConnectionError:
        return False
    return status == 200 and json.loads(body) == {"m.server": home.name + ":443"}


def verify_entry(home):
    # 入口刚起来时 frpc 可能还没注册完，frps 会先关连接；等到发现文件能取到为止。
    wait_for("entry " + home.label, lambda: _entry_ready(home))
    assert entry(home, "/.well-known/matrix/server") == {"m.server": home.name + ":443"}
    assert entry(home, "/.well-known/matrix/client") == {"m.homeserver": {"base_url": "https://" + home.name}}
    assert entry(home, "/.well-known/wonton")["endpoint_id"] == home.api("/_wonton/api/tunnel")["endpoint_id"]
    assert entry(home, "/_matrix/federation/v1/version")["server"]["name"]
    keys = entry(home, "/_matrix/key/v2/server")
    assert keys["server_name"] == home.name and keys["verify_keys"] and keys["signatures"][home.name]
    cert = run("openssl", "x509", "-in", home.install / "entry/certs/site.pem", "-noout", "-ext", "subjectAltName").stdout
    assert "DNS:" + home.name in cert and "DNS:*." + home.name in cert
    config = tomllib.loads((home.install / "secrets/tuwunel.toml").read_text())["global"]
    assert not config["allow_invalid_tls_certificates"]
    assert "allowed_remote_server_names_experimental" not in config
    status, _ = curl(home.cloud["network"], "https://" + home.name + "/_wonton/api/devices",
                     ca=home.cloud["ca"], dns=home.cloud["resolver"])
    assert status == 404
    print("PASS strict TLS, apex/wildcard SAN, discovery, federation version and signing keys: " + home.label, flush=True)


def pair(a, b):
    for home in (a, b):
        home.start_phone()
    code = b.phone_request("/_wonton/api/peers/code")["code"]
    try:
        assert a.phone_request("/_wonton/api/peers", {"code": code})["status"] == "awaiting_approval"
    except AssertionError:
        for home in (a, b):
            logs = home.compose("logs", "--no-color", "--tail", "120", check=False)
            print(logs.stdout + logs.stderr, flush=True)
        relay = run("bash", HERE / "dev.sh", "compose", "logs", "--no-color", "--tail", "120", "iroh", check=False)
        print(relay.stdout + relay.stderr, flush=True)
        raise
    requests = b.phone_request("/_wonton/api/peers")["requests"]
    if not any(row["server_name"] == a.name for row in requests):
        for home in (a, b):
            logs = home.compose("logs", "--no-color", "--tail", "120", check=False)
            print(logs.stdout + logs.stderr, flush=True)
        raise AssertionError("pairing request missing")
    request = next(row for row in requests if row["server_name"] == a.name)
    assert b.phone_request("/_wonton/api/peers/requests/" + request["id"] + "/approve", {})["approved"]
    for home, other in ((a, b), (b, a)):
        def connected():
            rows = home.phone_request("/_wonton/api/peers")["peers"]
            return any(row["server_name"] == other.name and row["status"] == "connected" for row in rows)
        wait_for("paired iroh", connected)
        def scoped():
            cfg = tomllib.loads((home.install / "fed/config.toml").read_text())["global"]
            return isinstance(cfg["proxy"], dict) and cfg["proxy"]["by_domain"][0]["include"] == [other.name]
        wait_for("paired proxy hot reload", scoped)
    print("PASS real owner devices approve pairing; only paired names use iroh", flush=True)


def suspend(a, b, alice, bob):
    context = ssl.create_default_context(cafile=a.cloud["ca"])
    raw = socket.create_connection((a.cloud["relay"], 443), timeout=10)
    with context.wrap_socket(raw, server_hostname=b.name) as connection:
        connection.sendall(f"GET /.well-known/matrix/server HTTP/1.1\r\nHost: {b.name}\r\nConnection: keep-alive\r\n\r\n".encode())
        reply = http.client.HTTPResponse(connection)
        reply.begin()
        assert reply.status == 200
        reply.read()
        run("bash", HERE / "dev.sh", "suspend", b.registration["home"], "local acceptance")
        try:
            curl(a.cloud["network"], "https://" + b.name + "/.well-known/matrix/server",
                 ca=a.cloud["ca"], dns=a.cloud["resolver"], resolve=b.name + ":443:" + a.cloud["relay"])
        except ConnectionError:
            pass
        else:
            raise AssertionError("Suspended home still accepts new public connections with cached DNS")
        connection.settimeout(65)
        assert connection.recv(1) == b"", "Established public connection survived suspension"
    print("PASS suspended home rejects cached-DNS public connections and closes existing connection", flush=True)
    # Restarting Tuwunel clears its outgoing connection pools and DNS cache. Pairing and room data remain.
    for home in (a, b):
        home.compose("restart", "tuwunel")
        home.compose("up", "--detach", "--wait", "--wait-timeout", "180")
    chat(a, b, alice, bob, "paired A to suspended B after cold TLS connections")
    chat(b, a, bob, alice, "suspended B to A over iroh")


def main():
    keep = "--keep" in sys.argv[1:]
    binary = Path(os.environ["WONTON_CORE_BINARY"]).resolve()
    home_binary = Path(os.environ["WONTON_HOME_BINARY"]).resolve()
    core_image = os.environ.get("WONTON_ENTRY_CORE_IMAGE", "wonton-core:e2e-main")
    assert binary.is_file() and home_binary.is_file()
    env = dict(os.environ, COMPOSE_PROJECT_NAME="wonton-fr-entry", WONTON_HOME_ZONE="homes.test",
               WONTON_HOME_BINARY=str(home_binary), WONTON_HOME_RUNTIME_IMAGE=os.environ.get("WONTON_HOME_RUNTIME_IMAGE", RUNTIME_IMAGE))
    for key in ["API", "RELAY", "DNS", "AUTH", "PEBBLE", "PEBBLE_MANAGEMENT"]:
        env["WONTON_ENTRY_" + key + "_PORT"] = str(free_port())
    root = Path(tempfile.mkdtemp(prefix="wonton-fr-entry-"))
    print("Smoke artifacts: " + str(root), flush=True)
    homes, acquired, cloud_started = [], False, False
    try:
        subprocess.run([str(SLOT), "acquire", "--project", "wonton-fr-entry", "fr-entry"], check=True)
        acquired = True
        env["WONTON_DEV_CORE_SUBNET"] = run(SLOT, "subnet", "fr-entry").stdout.strip()
        cloud_started = True
        print(run("bash", HERE / "dev.sh", "up", env=env, timeout=600).stdout, flush=True)
        values = dict(line.split("=", 1) for line in (HERE / ".runtime/stack.env").read_text().splitlines())
        cloud = {"zone": values["WONTON_HOME_ZONE"], "network": values["WONTON_ENTRY_NETWORK"],
                 "resolver": values["WONTON_ENTRY_RESOLVER_IP"], "relay": values["WONTON_ENTRY_RELAY_IP"],
                 "relay_port": values["WONTON_ENTRY_RELAY_PORT"], "subnet": values["WONTON_DEV_CORE_SUBNET"],
                 "ca": (HERE / ".runtime/ca/home-trust.pem").resolve()}
        for label in ("a", "b"):
            home = Home(label, root, cloud, binary, core_image)
            homes.append(home)
            home.install_home()
            verify_entry(home)
        a, b = homes
        alice, bob = account(a, "alice"), account(b, "bob")
        chat(a, b, alice, bob, "unpaired named A to B")
        chat(b, a, bob, alice, "unpaired named B to A")
        pair(a, b)
        suspend(a, b, alice, bob)
        print("PASS ADR-0051 entry acceptance", flush=True)
    finally:
        for home in reversed(homes):
            home.close()
        if cloud_started and not keep:
            print(run("bash", HERE / "dev.sh", "destroy", env=env).stdout, flush=True)
        if acquired and not keep:
            run(SLOT, "release", "fr-entry")
        if not keep:
            shutil.rmtree(root)
        print("Cleaned smoke artifacts" + ("" if keep else ", task containers, networks, volumes and slots"), flush=True)


if __name__ == "__main__":
    main()
