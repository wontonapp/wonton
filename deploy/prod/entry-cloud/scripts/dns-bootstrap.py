#!/usr/bin/env python3
"""Create the production zone and its fixed records in PowerDNS.

Idempotent: the zone is created only when absent, records are REPLACEd. Run it
after compose starts pdns and before the registrar NS switch.
"""
import json
import os
import sys
import time
import urllib.error
import urllib.request

PDNS = os.environ.get("WONTON_HOME_PDNS_URL", "http://127.0.0.1:8084").rstrip("/")
KEY_FILE = os.environ.get("WONTON_HOME_PDNS_KEY_FILE", "/opt/wonton-home/secrets/pdns-api-key")
DOMAIN = os.environ["WONTON_DOMAIN"]
PUBLIC_IP = os.environ["WONTON_PUBLIC_IP"]
PUSH = os.environ["WONTON_PUSH_HOST"]
RELAY = os.environ["WONTON_RELAY_HOST"]
NAMES = os.environ["WONTON_NAMES_HOST"]
OMARCHY = os.environ["WONTON_OMARCHY_HOST"]


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict]:
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(PDNS + path, data=data, method=method)
    request.add_header("X-API-Key", open(KEY_FILE).read().strip())
    request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status, json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or b"{}")


def wait() -> None:
    for _ in range(60):
        try:
            status, _ = call("GET", "/api/v1/servers/localhost")
        except OSError:
            status = 0
        if status == 200:
            return
        time.sleep(1)
    sys.exit("PowerDNS API did not become ready")


def rr(name: str, kind: str, ttl: int, contents: list[str]) -> dict:
    return {
        "name": name,
        "type": kind,
        "ttl": ttl,
        "changetype": "REPLACE",
        "records": [{"content": content, "disabled": False} for content in contents],
    }


def main() -> None:
    wait()
    zone_url = f"/api/v1/servers/localhost/zones/{DOMAIN}."
    status, _ = call("GET", zone_url)
    if status == 404:
        status, body = call("POST", "/api/v1/servers/localhost/zones", {
            "name": f"{DOMAIN}.",
            "kind": "Native",
            "nameservers": [f"ns-auth.{DOMAIN}."],
        })
        if status not in (200, 201):
            sys.exit(f"create zone failed: {status} {body}")

    rrsets = [
        rr(f"{DOMAIN}.", "NS", 3600, [f"ns-auth.{DOMAIN}.", f"ns2-auth.{DOMAIN}."]),
        rr(f"ns-auth.{DOMAIN}.", "A", 60, [PUBLIC_IP]),
        rr(f"ns2-auth.{DOMAIN}.", "A", 60, [PUBLIC_IP]),
        rr(f"{DOMAIN}.", "A", 60, [PUBLIC_IP]),
        rr(f"*.{DOMAIN}.", "A", 60, [PUBLIC_IP]),
        rr(f"www.{DOMAIN}.", "A", 60, [PUBLIC_IP]),
        rr(f"oma.{DOMAIN}.", "A", 60, [PUBLIC_IP]),
        rr(f"{PUSH}.", "A", 60, [PUBLIC_IP]),
        rr(f"{RELAY}.", "A", 60, [PUBLIC_IP]),
        rr(f"{NAMES}.", "A", 60, [PUBLIC_IP]),
        rr(f"{OMARCHY}.", "A", 60, [PUBLIC_IP]),
        rr(f"auth.{DOMAIN}.", "NS", 60, [f"ns-auth.{DOMAIN}."]),
        rr(f"{DOMAIN}.", "CAA", 60, [
            '0 issue "letsencrypt.org"',
            '0 issuewild "letsencrypt.org"',
        ]),
    ]
    status, body = call("PATCH", zone_url, {"rrsets": rrsets})
    if status != 204:
        sys.exit(f"seed records failed: {status} {body}")
    print(f"seeded {DOMAIN} with {len(rrsets)} rrsets")


if __name__ == "__main__":
    main()
