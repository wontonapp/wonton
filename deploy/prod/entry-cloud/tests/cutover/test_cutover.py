#!/usr/bin/env python3
"""Behaviour test for scripts/nginx-cutover.py and templates/nginx.

Runs only inside the container started by run.sh (it rewrites /etc/nginx and
/etc/letsencrypt there). The fixture mimics a host that certbot-nginx set up:
an apex site with a wildcard, push and relay vhosts, an HTTP-only personal
site that is the default server, the same site on 8080, and one unrelated site.
"""
import hashlib
import os
import re
import subprocess
import sys
import time
from pathlib import Path

SRC = Path("/src")
CUTOVER = [sys.executable, "-B", str(SRC / "scripts/nginx-cutover.py")]
ROOT = Path("/opt/wonton-home")
WORK = Path("/work")
AVAIL = Path("/etc/nginx/sites-available")
SITES = Path("/etc/nginx/sites-enabled")
RENEWAL = Path("/etc/letsencrypt/renewal")
WEBROOT = Path("/var/www/letsencrypt")
TOKEN = "tok-123"
DOMAINS = "push.example.com,relay.example.com,names.example.com,site.example,example.com,www.example.com,site.example.com"

ENV = f"""WONTON_DOMAIN=example.com
WONTON_PUBLIC_IP=192.0.2.10
WONTON_LISTEN_IP=127.0.0.1
WONTON_ACME_EMAIL=ops@example.com
WONTON_NAMES_HOST=names.example.com
WONTON_PUSH_HOST=push.example.com
WONTON_RELAY_HOST=relay.example.com
WONTON_OMARCHY_HOST=site.example
WONTON_SITE_ALIAS=site
WONTON_HOME_SYSTEM_DOMAINS={DOMAINS}
"""

CERTBOT_80 = """server {{
    if ($host = {name}) {{
        return 301 https://$host$request_uri;
    }} # managed by Certbot
{extra_if}
    listen 80;
    listen [::]:80;
    server_name {names};
    return 404; # managed by Certbot
}}
"""

TLS_443 = """
    listen [::]:443 ssl ipv6only=on; # managed by Certbot
    listen 443 ssl; # managed by Certbot
    ssl_certificate /etc/ssl/fixture/{cert}.crt; # managed by Certbot
    ssl_certificate_key /etc/ssl/fixture/{cert}.key; # managed by Certbot
}}
"""

FIXTURE = {
    "example.com": """server {
    server_name example.com www.example.com *.example.com;
    root /var/www/apex;
    index index.html;
    location / { try_files $uri $uri/ /index.html; }
    location ~ /\\. { deny all; }
""" + TLS_443.format(cert="apex") + CERTBOT_80.format(
        name="example.com", names="example.com www.example.com *.example.com",
        extra_if="    if ($host = www.example.com) {\n        return 301 https://$host$request_uri;\n    }"),
    "push.example.com": """server {
    server_name push.example.com;
    location / { return 200 "push\\n"; }
""" + TLS_443.format(cert="push").replace("ipv6only=on", "") + CERTBOT_80.format(
        name="push.example.com", names="push.example.com", extra_if=""),
    "relay.example.com": """server {
    server_name relay.example.com;
    location / { return 200 "relay\\n"; }
""" + TLS_443.format(cert="relay").replace("ipv6only=on", "") + CERTBOT_80.format(
        name="relay.example.com", names="relay.example.com", extra_if=""),
    "site": """server {
    listen 80 default_server;
    listen [::]:80 default_server;
    server_name site.example;
    root /var/www/site;
    location / { try_files $uri $uri/ /index.html; }
}
""",
    "site-alt": """server {
    listen 8080;
    listen [::]:8080;
    server_name _;
    root /var/www/site;
}
""",
    "blog": """server {
    listen 80;
    server_name blog.example.org;
    return 200 "blog\\n";
}
""",
}

RENEWAL_CONF = """version = 4.0.0
archive_dir = /etc/letsencrypt/archive/{name}
cert = /etc/letsencrypt/live/{name}/cert.pem
[renewalparams]
account = 0123456789abcdef
authenticator = nginx
installer = nginx
server = https://acme-v02.api.letsencrypt.org/directory
{hook}"""

ACCEPT = ["http:*.example.com", "http:default_server", "https:*.example.com", "https:default_server"]


def sh(*cmd, check=True):
    res = subprocess.run(cmd, text=True, capture_output=True)
    if check and res.returncode != 0:
        raise AssertionError(f"{cmd} -> {res.returncode}\n{res.stdout}\n{res.stderr}")
    return res


def cutover(*args, check=True):
    env_args = ["--frpc", str(ROOT / "frpc/system-frpc.toml")] if args[0] in ("check", "apply") else []
    return sh(*CUTOVER, *args, *env_args, check=check)


def snapshot():
    state = {}
    for base in (SITES, AVAIL, RENEWAL, Path("/etc/nginx/conf.d")):
        for p in sorted(base.iterdir()):
            state[str(p)] = ("link", os.readlink(p)) if p.is_symlink() else \
                hashlib.sha256(p.read_bytes()).hexdigest()
    return state


def curl(url, host=None, port=None, proxy=True):
    cmd = ["curl", "-sk", "--noproxy", "*", "--max-time", "5", "-o", "/dev/stderr", "-w", "%{http_code} %{redirect_url}"]
    if proxy:
        cmd.append("--haproxy-protocol")
    if host:
        cmd += ["--resolve", f"{host}:{port}:127.0.0.1"]
    res = subprocess.run(cmd + [url], text=True, capture_output=True)
    code, _, redirect = res.stdout.partition(" ")
    return code, redirect, res.stderr


def listening(port):
    out = sh("ss", "-lntpH").stdout
    return [row for row in out.splitlines() if row.split()[3].rsplit(":", 1)[-1] == str(port)]


def wait(pred, what):
    for _ in range(40):
        if pred():
            return
        time.sleep(0.25)
    raise AssertionError(f"timed out waiting for {what}")


def setup():
    WORK.mkdir()
    (WORK / ".env").write_text(ENV)
    (WORK / "compose.yaml").write_text("services: {}\n")
    bin_dir = Path("/usr/local/bin")
    (bin_dir / "systemctl").write_text(
        '#!/bin/sh\n# test shim: no systemd in the container\n'
        'case "$1 $2" in "reload nginx") exec nginx -s reload ;; "start nginx") exec nginx ;; esac\n'
        'echo "unexpected systemctl $*" >&2; exit 1\n')
    (bin_dir / "docker").write_text(
        '#!/bin/sh\n# test shim: records the call; "stop frps" frees the fake frps ports\n'
        'echo "$*" >> /work/docker.log\n'
        'case "$*" in *" stop frps system-frpc") kill "$(cat /work/frps.pid)"; rm /work/frps.pid ;; esac\n')
    for name in ("systemctl", "docker"):
        (bin_dir / name).chmod(0o755)
    fixture = Path("/etc/ssl/fixture")
    fixture.mkdir(parents=True)
    for cert, names in (("apex", "example.com,DNS:www.example.com"), ("push", "push.example.com"),
                        ("relay", "relay.example.com")):
        sh("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
           "-keyout", str(fixture / f"{cert}.key"), "-out", str(fixture / f"{cert}.crt"), "-days", "2",
           "-subj", f"/CN={cert}", "-addext", f"subjectAltName=DNS:{names}")
    for name, body in FIXTURE.items():
        (AVAIL / name).write_text(body)
        (SITES / name).symlink_to(AVAIL / name)
    for d, page in ((Path("/var/www/site"), "site"), (Path("/var/www/apex"), "apex"),
                    (Path("/var/www/omarchy"), "omarchy")):
        d.mkdir(parents=True, exist_ok=True)
        (d / "index.html").write_text(page + "\n")
    RENEWAL.mkdir(parents=True)
    for name in ("example.com", "push.example.com", "relay.example.com"):
        hook = "renew_hook = cp /etc/letsencrypt/live/relay.example.com/fullchain.pem /srv/relay/\n" \
            if name.startswith("relay") else ""
        (RENEWAL / f"{name}.conf").write_text(RENEWAL_CONF.format(name=name, hook=hook))
    sh(sys.executable, "-B", str(SRC / "prepare.py"), "--root", str(ROOT), "--env-file", str(WORK / ".env"))
    challenge = WEBROOT / ".well-known/acme-challenge"
    challenge.mkdir(parents=True)
    (challenge / TOKEN).write_text(TOKEN)
    sh("nginx", "-t")
    sh("nginx")
    wait(lambda: listening(80) and listening(443), "nginx on 80/443")


def accepts(stderr):
    return sorted(re.findall(r"--accept '([^']+)'", stderr))


def test_refuses_unrouted_names():
    before = snapshot()
    res = cutover("check", check=False)
    assert res.returncode == 1, res
    assert "blog.example.org" in res.stderr, res.stderr
    assert accepts(res.stderr) == sorted(ACCEPT + ["http:blog.example.org"]), res.stderr
    # Accepting everything else is not enough: blog still blocks apply.
    args = [a for t in ACCEPT for a in ("--accept", t)]
    res = cutover("apply", "--names-conf", str(ROOT / "nginx/names.conf"), "--realip", str(ROOT / "nginx/realip.conf"),
                  "--backup-dir", "/work/never", *args, check=False)
    assert res.returncode == 1 and "blog.example.org" in res.stderr, res
    assert not Path("/work/never").exists()
    assert snapshot() == before, "check/apply refused but changed files"
    # The operator decides blog goes elsewhere; only the four listed changes remain.
    (SITES / "blog").unlink()
    sh("nginx", "-s", "reload")
    res = cutover("check", check=False)
    assert accepts(res.stderr) == ACCEPT, res.stderr
    cutover("check", *args)
    # A public listener the script cannot rewrite in place is refused, not skipped.
    other = Path("/etc/nginx/conf.d/other.conf")
    other.write_text("server { listen 443 ssl; server_name push.example.com; "
                     "ssl_certificate /etc/ssl/fixture/push.crt; ssl_certificate_key /etc/ssl/fixture/push.key; }\n")
    res = cutover("check", *args, check=False)
    assert res.returncode == 1 and "outside /etc/nginx/sites-enabled" in res.stderr, res
    other.unlink()


def apply(backup):
    args = [a for t in ACCEPT for a in ("--accept", t)]
    return cutover("apply", "--names-conf", str(ROOT / "nginx/names.conf"),
                   "--realip", str(ROOT / "nginx/realip.conf"), "--backup-dir", backup, *args)


def test_apply_moves_only_listens(original):
    apply("/work/backup1")
    after = snapshot()
    for name in ("site-alt",):
        assert after[str(SITES / name)] == original[str(SITES / name)], f"{name} changed"
    for name in FIXTURE:
        assert after[str(AVAIL / name)] == original[str(AVAIL / name)], f"sites-available/{name} changed"
    alt = (SITES / "site-alt").read_text()
    assert "listen 8080;" in alt and "listen [::]:8080;" in alt
    wait(lambda: not listening(80) and not listening(443), "80/443 released")
    assert listening(4080) and listening(4443)
    dump = sh("nginx", "-T").stdout
    live = [l for l in dump.splitlines() if re.match(r"\s*listen\s", l)]
    assert all(re.search(r"127\.0\.0\.1:(4080|4443) .*proxy_protocol|[ :]8080;", l) for l in live), live
    # ACME through the moved vhosts: 80 still redirects (certbot `if`), 443 serves the webroot,
    # also where a regex location would otherwise deny dot paths.
    for host in ("example.com", "www.example.com", "push.example.com", "relay.example.com"):
        code, redirect, _ = curl(f"http://{host}:4080/.well-known/acme-challenge/{TOKEN}", host, 4080)
        assert code == "301" and redirect == f"https://{host}/.well-known/acme-challenge/{TOKEN}", (host, code)
        code, _, body = curl(f"https://{host}:4443/.well-known/acme-challenge/{TOKEN}", host, 4443)
        assert (code, body) == ("200", TOKEN), (host, code, body)
    code, _, body = curl("https://push.example.com:4443/", "push.example.com", 4443)
    assert (code, body) == ("200", "push\n")
    code, _, body = curl("http://site.example:4080/deep/link", "site.example", 4080)
    assert (code, body) == ("200", "site\n"), "SPA fallback of the personal site lost"


def test_renewal_check_catches_lost_hook():
    for name in ("push.example.com", "relay.example.com"):
        conf = RENEWAL / f"{name}.conf"
        text = conf.read_text().replace("authenticator = nginx", "authenticator = webroot")
        if name.startswith("relay"):
            text = text.replace("renew_hook", "# renew_hook")
        conf.write_text(text)
    sh(*CUTOVER, "renewal-check", "--backup-dir", "/work/backup1", "--cert-name", "push.example.com")
    res = sh(*CUTOVER, "renewal-check", "--backup-dir", "/work/backup1", "--cert-name", "relay.example.com",
             check=False)
    assert res.returncode == 1 and "renew_hook changed" in res.stderr, res


def fake_frps():
    proc = subprocess.Popen([sys.executable, "-c",
                             "import socket,time\n"
                             "s=[socket.create_server(('0.0.0.0',p)) for p in (80,443)]\n"
                             "time.sleep(3600)"], start_new_session=True)
    (WORK / "frps.pid").write_text(str(proc.pid))
    wait(lambda: listening(80) and listening(443), "fake frps")
    return proc


def test_rollback_restores_everything(original):
    proc = fake_frps()
    sh(*CUTOVER, "rollback", "--backup-dir", "/work/backup1", "--compose-dir", str(WORK))
    proc.wait(timeout=5)
    assert "stop frps system-frpc" in (WORK / "docker.log").read_text()
    assert snapshot() == original, "rollback left differences"
    assert all("nginx" in row for row in listening(80) + listening(443))
    code, _, body = curl("http://127.0.0.1/", proxy=False)
    assert (code, body) == ("200", "site\n"), "default server not back"
    code, _, body = curl("https://push.example.com/", "push.example.com", 443, proxy=False)
    assert (code, body) == ("200", "push\n")


def test_swap_installs_template_and_undoes():
    apply("/work/backup2")
    sh(*CUTOVER, "swap", "--backup-dir", "/work/backup3", "--remove", "example.com",
       "--install", str(ROOT / "nginx/apex.conf"))
    wait(lambda: curl("https://example.com:4443/", "example.com", 4443)[2] == "wonton\n", "apex template")
    for port, scheme in ((4080, "http"), (4443, "https")):
        code, _, body = curl(f"{scheme}://example.com:{port}/.well-known/acme-challenge/{TOKEN}", "example.com", port)
        assert (code, body) == ("200", TOKEN), (port, code, body)
    assert curl("http://www.example.com:4080/x", "www.example.com", 4080)[:2] == ("301", "https://www.example.com/x")
    sh(*CUTOVER, "rollback", "--nginx-only", "--backup-dir", "/work/backup3")
    wait(lambda: curl("https://example.com:4443/", "example.com", 4443)[2] == "apex\n", "old apex back")


def test_fresh_host_templates():
    """The rendered templates alone (a fresh host): ACME reachable on 80 and 443 for every name."""
    sh("nginx", "-s", "stop")
    wait(lambda: not listening(4080), "nginx stopped")
    for p in SITES.iterdir():
        p.unlink()
    for p in (ROOT / "nginx").glob("*.conf"):
        if p.name != "realip.conf":
            (SITES / p.name).symlink_to(p)
    sh("nginx", "-t")
    sh("nginx")
    wait(lambda: listening(4443), "template nginx")
    for host in ("example.com", "www.example.com", "push.example.com", "relay.example.com", "names.example.com"):
        for port, scheme in ((4080, "http"), (4443, "https")):
            code, _, body = curl(f"{scheme}://{host}:{port}/.well-known/acme-challenge/{TOKEN}", host, port)
            assert (code, body) == ("200", TOKEN), (host, port, code, body)
        assert curl(f"http://{host}:4080/x", host, 4080)[:2] == ("301", f"https://{host}/x"), host
    code, _, body = curl("https://example.com:4443/", "example.com", 4443)
    assert (code, body) == ("404", "wonton\n")
    code, _, body = curl("http://site.example:4080/a/b", "site.example", 4080)
    assert (code, body) == ("200", "omarchy\n")


def main():
    setup()
    os.environ["WONTON_HOME_SYSTEM_DOMAINS"] = DOMAINS
    test_refuses_unrouted_names()
    original = snapshot()
    print("ok  refuses unrouted names, changes nothing")
    steps = [("apply moves only 80/443 listens", lambda: test_apply_moves_only_listens(original)),
             ("renewal-check catches a lost hook", test_renewal_check_catches_lost_hook),
             ("rollback restores files, ports, renewal", lambda: test_rollback_restores_everything(original)),
             ("swap installs apex template, undo", test_swap_installs_template_and_undoes),
             ("fresh-host templates serve ACME on 80/443", test_fresh_host_templates)]
    for name, step in steps:
        step()
        print(f"ok  {name}")
    print("all cutover tests passed")


if __name__ == "__main__":
    main()
