#!/usr/bin/env python3
"""Render the production entry cloud on the operator host.

Only writes under `--root` (default /opt/wonton-home) and the placeholder
certificate directories; it never changes DNS or starts a service.
"""
import argparse
import os
import re
import secrets
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
TEMPLATES = HERE / "templates"


def load_env(path: Path) -> dict[str, str]:
    values = dict(os.environ)
    if path.exists():
        for raw in path.read_text().splitlines():
            line = raw.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            values.setdefault(key.strip(), value.strip())
    return values


def need(values: dict[str, str], key: str) -> str:
    value = values.get(key, "").strip()
    if not value:
        sys.exit(f"missing {key} (set it in .env)")
    return value


def write(path: Path, text: str, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


def secret(path: Path, mode: int = 0o600) -> str:
    if path.exists():
        return path.read_text().strip()
    value = secrets.token_hex(32)
    write(path, value + "\n", mode)
    return value


def render(template: Path, dest: Path, values: dict[str, str], mode: int = 0o644) -> None:
    text = template.read_text()
    # Empty is meaningful for the optional omarchy alias when it equals the main name.
    rendered = re.sub(
        r"@([A-Z0-9_]+)@",
        lambda m: values.get(m.group(1), "") if m.group(1) == "OMARCHY_ALT_DOMAIN" else need(values, m.group(1)),
        text,
    )
    write(dest, rendered, mode)


def run(*args: str) -> None:
    subprocess.run(args, check=True, stdout=subprocess.DEVNULL)


def ensure_ca(root: Path) -> tuple[Path, Path]:
    ca_key, ca_crt = root / "secrets/control-ca.key", root / "frps/ca.crt"
    if ca_key.exists() != ca_crt.exists():
        sys.exit(f"control CA incomplete: remove both {ca_key} and {ca_crt}, then rerun")
    if not ca_key.exists():
        run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
            "-nodes", "-keyout", str(ca_key), "-out", str(ca_crt), "-days", "3650",
            "-subj", "/CN=wonton entry control CA")
        ca_key.chmod(0o600)
    # certbot writes 0600/0644 under live, but nginx workers read the copy.
    ca_crt.chmod(0o644)
    return ca_key, ca_crt


def ensure_leaf(ca_key: Path, ca_crt: Path, key: Path, cert: Path, host: str) -> None:
    if key.exists() and cert.exists():
        return
    csr = key.with_suffix(".csr")
    run("openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
        "-keyout", str(key), "-out", str(csr), "-subj", f"/CN={host}",
        "-addext", f"subjectAltName=DNS:{host}")
    run("openssl", "x509", "-req", "-in", str(csr), "-CA", str(ca_crt), "-CAkey", str(ca_key),
        "-CAcreateserial", "-days", "397", "-copy_extensions", "copy", "-out", str(cert))
    csr.unlink(missing_ok=True)


def ensure_placeholder_cert(host: str, le_root: Path) -> None:
    live = le_root / host
    if (live / "fullchain.pem").exists():
        return
    key = live / "privkey.pem"
    cert = live / "fullchain.pem"
    write(key, "", 0o600)
    write(cert, "", 0o644)
    run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
        "-nodes", "-keyout", str(key), "-out", str(cert), "-days", "7",
        "-subj", f"/CN={host}", "-addext", f"subjectAltName=DNS:{host}")
    key.chmod(0o600)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=os.environ.get("WONTON_ENTRY_ROOT", "/opt/wonton-home"))
    parser.add_argument("--env-file", default=str(HERE / ".env"))
    args = parser.parse_args()

    values = load_env(Path(args.env_file))
    values["DOMAIN"] = need(values, "WONTON_DOMAIN")
    values["PUBLIC_IP"] = need(values, "WONTON_PUBLIC_IP")
    values["LISTEN_IP"] = values.get("WONTON_LISTEN_IP") or values["PUBLIC_IP"]
    for key, short in (("WONTON_PUSH_HOST", "PUSH_HOST"), ("WONTON_RELAY_HOST", "RELAY_HOST"),
                       ("WONTON_NAMES_HOST", "NAMES_HOST"), ("WONTON_OMARCHY_HOST", "OMARCHY_HOST")):
        values[short] = need(values, key)
    values["PUBLIC_SERVICE_URL"] = f"https://{values['NAMES_HOST']}"
    values["WONTON_CERTS_DIR"] = values.get("WONTON_CERTS_DIR") or str(Path(args.root) / "certs")
    values["SITE_ALIAS"] = need(values, "WONTON_SITE_ALIAS")
    values["OMARCHY_ALIAS_DOMAIN"] = f"{values['SITE_ALIAS']}.{values['DOMAIN']}"
    values["OMARCHY_ALT_DOMAIN"] = (
        "" if values["OMARCHY_HOST"] == values["OMARCHY_ALIAS_DOMAIN"] else values["OMARCHY_ALIAS_DOMAIN"]
    )
    # Filled from the generated secret below; do not generate a second value.

    root = Path(args.root)
    for sub in ("frps", "frpc", "pdns", "pdns-conf", "dnsdist", "acme-dns", "acme-dns-data",
                "state", "secrets", "bootstrap", "nginx", "certs", "scripts"):
        (root / sub).mkdir(parents=True, exist_ok=True)
        (root / sub).chmod(0o755 if sub != "secrets" else 0o700)

    pdns_key = secret(root / "secrets/pdns-api-key")
    system_token = secret(root / "secrets/system-frpc-token")
    values["FRPS_METRICS_PASSWORD"] = secret(root / "secrets/frps-metrics-password")
    values["PDNS_API_KEY"] = pdns_key
    values["SYSTEM_TOKEN"] = system_token
    values["WONTON_ENTRY_ROOT"] = str(root)

    ca_key, ca_crt = ensure_ca(root)
    ensure_leaf(ca_key, ca_crt, root / "frps/frps.key", root / "frps/frps.crt",
                need(values, "WONTON_RELAY_HOST"))

    render(TEMPLATES / "frps.toml.tmpl", root / "frps/frps.toml", values, 0o640)
    render(TEMPLATES / "system-frpc.toml.tmpl", root / "frpc/system-frpc.toml", values, 0o640)
    render(TEMPLATES / "pdns.conf.tmpl", root / "pdns-conf/pdns.conf", values, 0o640)
    render(TEMPLATES / "dnsdist.conf.tmpl", root / "dnsdist/dnsdist.conf", values, 0o644)
    render(TEMPLATES / "acme-dns.cfg.tmpl", root / "acme-dns/config.cfg", values, 0o640)
    render(TEMPLATES / "bootstrap/install.sh", root / "bootstrap/install.sh", values, 0o644)

    certs_dir = Path(values["WONTON_CERTS_DIR"])
    for host in (values["WONTON_PUSH_HOST"], values["WONTON_RELAY_HOST"],
                 values["WONTON_NAMES_HOST"], values["DOMAIN"],
                 f"www.{values['DOMAIN']}"):
        ensure_placeholder_cert(host, certs_dir)

    for template in sorted((TEMPLATES / "nginx").glob("*.conf.tmpl")):
        dest = root / "nginx" / template.name.removesuffix(".tmpl")
        render(template, dest, values, 0o644)

    write(root / "secrets/prepare.env", "\n".join(
        f"{k}={values[k]}" for k in
        ("WONTON_DOMAIN", "WONTON_PUBLIC_IP", "WONTON_RELAY_HOST", "WONTON_NAMES_HOST")
    ) + "\n", 0o600)
    print(f"prepared {root}")
    print("next: docker compose --env-file .env config -q, then run scripts/dns-bootstrap.py")


if __name__ == "__main__":
    main()
