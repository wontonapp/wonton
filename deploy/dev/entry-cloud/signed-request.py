#!/usr/bin/env python3
"""Send an actual endpoint-signed management request without Python crypto packages."""
import argparse
import hashlib
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time


def openssl(*args):
    return subprocess.check_output(["openssl", *map(str, args)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("method", choices=["POST", "GET", "PUT", "DELETE"])
    parser.add_argument("path")
    parser.add_argument("--key", type=Path, required=True, help="Home-only Ed25519 PEM key")
    parser.add_argument("--body", type=Path, help="Exact raw JSON request body file")
    parser.add_argument("--output", type=Path, help="Store response (may contain credentials)")
    args = parser.parse_args()
    if not args.path.startswith("/") or any(char in args.path for char in "?#\r\n"):
        parser.error("Use an absolute management path without query or fragment")
    runtime = Path(__file__).resolve().parent / ".runtime"
    settings = dict(line.split("=", 1) for line in (runtime / "stack.env").read_text().splitlines())
    hostname = "names." + settings["WONTON_HOME_ZONE"]
    port = settings["WONTON_ENTRY_API_PORT"]
    body = args.body.read_bytes() if args.body else b""
    public = openssl("pkey", "-in", args.key, "-pubout", "-outform", "DER")
    if len(public) != 44 or public[:12].hex() != "302a300506032b6570032100":
        parser.error("--key must be Ed25519")
    timestamp, nonce = str(int(time.time())), secrets.token_hex(16)
    canonical = f"wonton-home-v1\n{args.method}\n{args.path}\n{timestamp}\n{nonce}\n{hashlib.sha256(body).hexdigest()}"
    with tempfile.TemporaryDirectory(prefix="wonton-home-sign-") as temporary:
        message = Path(temporary) / "message"
        message.write_bytes(canonical.encode())
        signature = openssl("pkeyutl", "-sign", "-rawin", "-inkey", args.key, "-in", message).hex()
        command = ["curl", "--fail-with-body", "--silent", "--show-error", "--noproxy", "*",
                   "--cacert", str(runtime / "ca/control-ca.pem"),
                   "--resolve", f"{hostname}:{port}:127.0.0.1", "--request", args.method,
                   "--header", f"X-Wonton-Endpoint: {public[-32:].hex()}",
                   "--header", f"X-Wonton-Timestamp: {timestamp}",
                   "--header", f"X-Wonton-Nonce: {nonce}",
                   "--header", f"X-Wonton-Signature: {signature}"]
        if args.body:
            command += ["--header", "Content-Type: application/json", "--data-binary", "@" + str(args.body)]
        command += [f"https://{hostname}:{port}{args.path}"]
        if args.output:
            fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, "wb") as output:
                subprocess.run(command, stdout=output, check=True)
        else:
            subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
