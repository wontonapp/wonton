#!/usr/bin/env python3
"""Seed infrastructure records without owning any per-home records."""
import json
import os
from pathlib import Path
import time
from urllib.error import HTTPError, URLError
from urllib.request import ProxyHandler, Request, build_opener

# 容器刚启动时 Docker 可能还没配好它的 veth 路由，第一下会 ENETUNREACH；PDNS 也可能还在初始化。
STARTUP_WAIT = 60


def request(method, path, value=None):
    body = None if value is None else json.dumps(value).encode()
    req = Request(
        f"http://{os.environ['WONTON_ENTRY_PDNS_IP']}:8081/api/v1/servers/localhost{path}",
        data=body,
        method=method,
        headers={
            "X-API-Key": Path("/pdns-key").read_text().strip(),
            "Content-Type": "application/json",
        },
    )
    deadline = time.monotonic() + STARTUP_WAIT
    while True:
        try:
            with build_opener(ProxyHandler({})).open(req, timeout=10) as response:
                return response.read()
        except HTTPError:
            raise
        except (URLError, OSError) as error:
            if time.monotonic() >= deadline:
                raise
            print(f"PDNS not reachable yet ({error}); retrying", flush=True)
            time.sleep(1)


def main():
    zone = os.environ["WONTON_HOME_ZONE"] + "."
    try:
        request("GET", f"/zones/{zone}")
    except HTTPError as error:
        if error.code != 404:
            raise
        request("POST", "/zones", {
            "name": zone,
            "kind": "Native",
            "nameservers": ["ns." + zone],
        })
    request("PATCH", f"/zones/{zone}", json.loads(Path("/records.json").read_text()))


if __name__ == "__main__":
    main()
