#!/usr/bin/env python3
"""Generate only isolated .test infrastructure configuration and control-plane keys."""
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import subprocess

ROOT = Path(__file__).resolve().parent
RUNTIME = ROOT / ".runtime"


def write(relative, text, mode=0o644):
    path = RUNTIME / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


def openssl(*args):
    subprocess.run(["openssl", *map(str, args)], check=True)


def certificate(name, hostname):
    directory = RUNTIME / name
    directory.mkdir(parents=True, exist_ok=True)
    key, cert = directory / "key.pem", directory / "cert.pem"
    if key.exists() and cert.exists():
        return
    openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", key,
            "-out", directory / "request.pem", "-subj", f"/CN={hostname}")
    write(f"{name}/extensions.cnf", "basicConstraints=critical,CA:FALSE\n"
          "keyUsage=critical,digitalSignature,keyEncipherment\n"
          f"extendedKeyUsage=serverAuth\nsubjectAltName=DNS:{hostname},IP:127.0.0.1\n")
    openssl("x509", "-req", "-in", directory / "request.pem", "-CA", RUNTIME / "ca/ca.pem",
            "-CAkey", RUNTIME / "secrets/control-ca-key.pem", "-set_serial", str(secrets.randbits(128)),
            "-days", "30", "-sha256", "-extfile", directory / "extensions.cnf", "-out", cert)
    key.chmod(0o600)
    cert.chmod(0o644)
    (directory / "request.pem").unlink()
    (directory / "extensions.cnf").unlink()


def dns_configuration(values):
    zone = values["WONTON_HOME_ZONE"]
    ip = lambda service: values[f"WONTON_ENTRY_{service}_IP"]
    pdns_key = (RUNTIME / "secrets/pdns-key").read_text().strip()
    write("pdns/pdns.conf", f"""local-address=0.0.0.0
launch=gsqlite3
gsqlite3-database=/var/lib/powerdns/pdns.sqlite3
api=yes
api-key={pdns_key}
webserver=yes
webserver-address=0.0.0.0
webserver-port=8081
webserver-allow-from={values['WONTON_DEV_CORE_SUBNET']},127.0.0.0/8
security-poll-suffix=
""")
    write("dnsdist/dnsdist.conf", f"""setLocal('0.0.0.0:53')
setACL({{'{values['WONTON_DEV_CORE_SUBNET']}', '127.0.0.0/8'}})
newServer({{address='{ip('PDNS')}:53', pool='authoritative', checkName='{zone}.', checkType='SOA'}})
newServer({{address='{ip('ACME_DNS')}:53', pool='acme', checkName='auth.{zone}.', checkType='SOA'}})
addAction(QNameSuffixRule('auth.{zone}.'), PoolAction('acme'))
addAction(QNameSuffixRule('{zone}.'), PoolAction('authoritative'))
addAction(AllRule(), RCodeAction(DNSRCode.REFUSED))
setSecurityPollSuffix('')
""")
    write("resolver/recursor.conf", f"""local-address=0.0.0.0
local-port=53
allow-from={values['WONTON_DEV_CORE_SUBNET']},127.0.0.0/8
forward-zones=.={ip('DNSDIST')}:53
dont-query=
dnssec=off
hint-file=no-refresh
max-cache-ttl=0
max-negative-ttl=0
packetcache-ttl=0
packetcache-negative-ttl=0
security-poll-suffix=
""")
    write("acme-dns/config.cfg", f"""[general]
listen = "0.0.0.0:53"
protocol = "both4"
domain = "auth.{zone}"
nsname = "auth.{zone}"
nsadmin = "admin.{zone}"
records = ["auth.{zone}. A {ip('DNSDIST')}", "auth.{zone}. NS auth.{zone}."]
debug = false
[database]
engine = "sqlite3"
connection = "/var/lib/acme-dns/acme-dns.db"
[api]
ip = "0.0.0.0"
port = "8080"
tls = "none"
disable_registration = false
corsorigins = []
use_header = false
[logconfig]
loglevel = "info"
logtype = "stdout"
logformat = "text"
""")


def control_configuration(values):
    zone = values["WONTON_HOME_ZONE"]
    ip = lambda service: values[f"WONTON_ENTRY_{service}_IP"]
    password = (RUNTIME / "secrets/frps-password").read_text().strip()
    write("frps/frps.toml", f"""bindAddr = "0.0.0.0"
proxyBindAddr = "0.0.0.0"
bindPort = 443
vhostHTTPSPort = 443
transport.tls.force = true
transport.tls.certFile = "/config/cert.pem"
transport.tls.keyFile = "/config/key.pem"
transport.heartbeatTimeout = 90
auth.method = "token"
auth.token = ""
webServer.addr = "127.0.0.1"
webServer.port = 7500
webServer.user = "wonton"
webServer.password = "{password}"
enablePrometheus = true
log.to = "console"
log.level = "info"
log.disablePrintColor = true
detailedErrorsToClient = false
[[httpPlugins]]
name = "home-policy"
addr = "127.0.0.1:8081"
path = "/frp"
ops = ["Login", "NewProxy", "NewUserConn", "Ping", "CloseProxy"]
""")
    write("api/haproxy.cfg", f"""global
  log stdout format raw local0
  maxconn 2048
defaults
  log global
  mode http
  timeout connect 5s
  timeout client 30s
  timeout server 30s
frontend names
  bind :8443 ssl crt /usr/local/etc/haproxy/api.pem ssl-min-ver TLSv1.2
  acl correct_host hdr(host) -i names.{zone} names.{zone}:8443 names.{zone}:{values['WONTON_ENTRY_API_PORT']}
  http-request deny deny_status 421 unless correct_host
  http-request deny deny_status 404 if {{ path_beg /frp }}
  http-request deny deny_status 404 if {{ path /acme/register }}
  default_backend home
backend home
  option httpchk GET /health
  server home {ip('RELAY')}:8080 check
""")
    write("iroh/relay.toml", f"""http_bind_addr = "0.0.0.0:3342"
[tls]
cert_mode = "Manual"
manual_cert_path = "/config/cert.pem"
manual_key_path = "/config/key.pem"
""")
    write("pebble/pebble.json", json.dumps({"pebble": {
        "listenAddress": "0.0.0.0:14000", "managementListenAddress": "0.0.0.0:15000",
        "certificate": "/config/cert.pem", "privateKey": "/config/key.pem",
        "httpPort": 5002, "tlsPort": 5001, "externalAccountBindingRequired": False,
    }}, indent=2) + "\n")


def records(values):
    zone = values["WONTON_HOME_ZONE"] + "."
    def rr(name, kind, content):
        return {"name": name + zone, "type": kind, "ttl": 1,
                "changetype": "REPLACE", "records": [{"content": content, "disabled": False}]}
    data = [rr("ns.", "A", values["WONTON_ENTRY_DNSDIST_IP"]),
            rr("auth.", "NS", "auth." + zone),
            rr("auth.", "A", values["WONTON_ENTRY_DNSDIST_IP"])]
    for label, service in [("relay", "RELAY"), ("iroh", "IROH"), ("names", "API"), ("pebble", "PEBBLE")]:
        data.append(rr(label + ".", "A", values[f"WONTON_ENTRY_{service}_IP"]))
    write("dns-records.json", json.dumps({"rrsets": data}, indent=2) + "\n")


def settings():
    values = {}
    previous = RUNTIME / "stack.env"
    if previous.exists():
        values.update(line.split("=", 1) for line in previous.read_text().splitlines() if line)
    services = ["DNSDIST", "PDNS", "ACME_DNS", "RESOLVER", "PEBBLE", "RELAY", "API", "BOOTSTRAP", "IROH"]
    ports = {"API": 8443, "RELAY": 9443, "DNS": 15353, "AUTH": 15354,
             "PEBBLE": 14000, "PEBBLE_MANAGEMENT": 15000}
    allowed = {"WONTON_HOME_ZONE", "WONTON_DEV_CORE_SUBNET", "COMPOSE_PROJECT_NAME", "WONTON_ENTRY_NETWORK",
               "WONTON_HOME_BINARY", "WONTON_HOME_RUNTIME_IMAGE"}
    allowed.update(f"WONTON_ENTRY_{name}_IP" for name in services)
    allowed.update(f"WONTON_ENTRY_{name}_PORT" for name in ports)
    values = {key: value for key, value in values.items() if key in allowed}
    values.update({key: value for key, value in os.environ.items() if key in allowed})
    if values.get("WONTON_HOME_BINARY"):
        binary = Path(values["WONTON_HOME_BINARY"])
        if not binary.is_absolute() or not binary.is_file() or not re.fullmatch(r"/[a-zA-Z0-9_./+-]+", str(binary)):
            raise SystemExit("WONTON_HOME_BINARY must be an existing absolute ELF path without shell metacharacters")
    if values.get("WONTON_HOME_RUNTIME_IMAGE") and not re.fullmatch(
            r"[a-zA-Z0-9][a-zA-Z0-9./_:+-]*@sha256:[0-9a-f]{64}", values["WONTON_HOME_RUNTIME_IMAGE"]):
        raise SystemExit("WONTON_HOME_RUNTIME_IMAGE must be a sha256-pinned image reference")
    if not values.get("WONTON_DEV_CORE_SUBNET"):
        raise SystemExit("Acquire a stack slot and export WONTON_DEV_CORE_SUBNET first")
    network = ipaddress.ip_network(values["WONTON_DEV_CORE_SUBNET"], strict=True)
    if network.version != 4 or network.prefixlen != 24 or not network.is_private:
        raise SystemExit("WONTON_DEV_CORE_SUBNET must be a private IPv4 /24 from stack-slot")
    values.setdefault("COMPOSE_PROJECT_NAME", "wonton-entry-cloud")
    values.setdefault("WONTON_HOME_ZONE", "homes.test")
    zone = values["WONTON_HOME_ZONE"]
    if not zone.endswith(".test") or not re.fullmatch(r"[a-z0-9]+(?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9]+(?:[a-z0-9-]*[a-z0-9])?)+", zone):
        raise SystemExit("WONTON_HOME_ZONE must be a lowercase .test DNS zone")
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", values["COMPOSE_PROJECT_NAME"]):
        raise SystemExit("Invalid Compose project name")
    values.setdefault("WONTON_ENTRY_NETWORK", values["COMPOSE_PROJECT_NAME"] + "-entry")
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]*", values["WONTON_ENTRY_NETWORK"]):
        raise SystemExit("Invalid Docker network name")
    for offset, service in enumerate(services, 10):
        key = f"WONTON_ENTRY_{service}_IP"
        values.setdefault(key, str(network.network_address + offset))
        address = ipaddress.ip_address(values[key])
        if address not in network or address in {network.network_address, network.broadcast_address, network.network_address + 1}:
            raise SystemExit(f"{key} must be a usable address inside WONTON_DEV_CORE_SUBNET (not gateway .1)")
    addresses = [value for key, value in values.items() if key.endswith("_IP")]
    if len(addresses) != len(set(addresses)):
        raise SystemExit("Cloud service addresses must be distinct")
    for name, default in ports.items():
        key = f"WONTON_ENTRY_{name}_PORT"
        values.setdefault(key, str(default))
        if not values[key].isdigit() or not 1024 <= int(values[key]) <= 65535:
            raise SystemExit(f"{key} must be an unprivileged TCP/UDP port")
    published = [values[f"WONTON_ENTRY_{name}_PORT"] for name in ports]
    if len(published) != len(set(published)):
        raise SystemExit("Cloud published ports must be distinct")
    identity = RUNTIME / "identity"
    if identity.exists() and identity.read_text() != zone:
        raise SystemExit("Changing the zone requires removing this stack's volumes and .runtime")
    write("identity", zone)
    return values


def main():
    values = settings()
    ca = RUNTIME / "ca"
    ca.mkdir(parents=True, exist_ok=True)
    (RUNTIME / "secrets").mkdir(parents=True, exist_ok=True)
    if not (ca / "ca.pem").exists():
        openssl("req", "-x509", "-newkey", "rsa:3072", "-nodes", "-sha256", "-days", "30",
                "-keyout", RUNTIME / "secrets/control-ca-key.pem", "-out", ca / "ca.pem", "-subj", "/CN=Wonton local entry control CA",
                "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        (RUNTIME / "secrets/control-ca-key.pem").chmod(0o600)
    for name in ["pdns-key", "frps-password"]:
        path = RUNTIME / "secrets" / name
        if not path.exists():
            write(f"secrets/{name}", secrets.token_hex(32) + "\n", 0o600)
    certificate("api", "names." + values["WONTON_HOME_ZONE"])
    certificate("frps", "relay." + values["WONTON_HOME_ZONE"])
    certificate("iroh", "iroh." + values["WONTON_HOME_ZONE"])
    certificate("pebble", "pebble." + values["WONTON_HOME_ZONE"])
    write("api/api.pem", (RUNTIME / "api/key.pem").read_text() + (RUNTIME / "api/cert.pem").read_text(), 0o600)
    dns_configuration(values)
    control_configuration(values)
    records(values)
    (RUNTIME / "secrets").chmod(0o700)
    for directory in [RUNTIME, RUNTIME / "ca", RUNTIME / "pdns", RUNTIME / "dnsdist", RUNTIME / "resolver"]:
        directory.chmod(0o755)
    write("stack.env", "".join(f"{key}={value}\n" for key, value in sorted(values.items())), 0o600)
    write("ca/control-ca.pem", (ca / "ca.pem").read_text())
    (ca / "control-ca.pem").chmod(0o644)
    print(f"Prepared {values['COMPOSE_PROJECT_NAME']}; public trust file: {ca / 'control-ca.pem'}")


if __name__ == "__main__":
    main()
