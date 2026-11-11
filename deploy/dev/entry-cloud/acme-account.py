#!/usr/bin/env python3
"""Create a real Pebble account in lego's storage before the CAA binding; no order/leaf."""
import argparse
import base64
import json
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import urlparse


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def curl(runtime, settings, url, *arguments):
    zone = settings["WONTON_HOME_ZONE"]
    if urlparse(url).hostname != "pebble." + zone or urlparse(url).port != 14000:
        raise ValueError("ACME resources must remain on this local Pebble")
    return subprocess.check_output([
        "curl", "--fail-with-body", "--silent", "--show-error", "--noproxy", "*",
        "--cacert", str(runtime / "ca/control-ca.pem"),
        "--connect-to", f"pebble.{zone}:14000:127.0.0.1:{settings['WONTON_ENTRY_PEBBLE_PORT']}",
        *arguments, url,
    ])


def register(runtime, settings, key, email, directory):
    modulus = subprocess.check_output(["openssl", "rsa", "-in", str(key), "-noout", "-modulus"], text=True)
    number = bytes.fromhex(modulus.strip().split("=", 1)[1])
    jwk = {"kty": "RSA", "n": b64(number), "e": "AQAB"}
    with tempfile.TemporaryDirectory(prefix="wonton-acme-account-") as temporary:
        headers = Path(temporary) / "headers"
        curl(runtime, settings, directory["newNonce"], "--head", "--dump-header", str(headers))
        nonce = next(line.split(":", 1)[1].strip() for line in headers.read_text().splitlines()
                     if line.lower().startswith("replay-nonce:"))
        protected = b64(json.dumps({"alg": "RS256", "jwk": jwk, "nonce": nonce,
                                    "url": directory["newAccount"]}, separators=(",", ":")).encode())
        payload = b64(json.dumps({"termsOfServiceAgreed": True, "contact": ["mailto:" + email]},
                                 separators=(",", ":")).encode())
        message = Path(temporary) / "message"
        message.write_text(protected + "." + payload)
        signature = subprocess.check_output(["openssl", "dgst", "-sha256", "-sign", str(key), str(message)])
        request = Path(temporary) / "request.json"
        request.write_text(json.dumps({"protected": protected, "payload": payload, "signature": b64(signature)}))
        response = curl(runtime, settings, directory["newAccount"], "--dump-header", str(headers),
                        "--header", "Content-Type: application/jose+json", "--data-binary", "@" + str(request))
        location = next(line.split(":", 1)[1].strip() for line in headers.read_text().splitlines()
                        if line.lower().startswith("location:"))
        return {"email": email, "registration": {"uri": location, "body": json.loads(response)}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", type=Path, required=True, help="Home-owned acceptance directory")
    parser.add_argument("--email", required=True)
    args = parser.parse_args()
    runtime = Path(__file__).resolve().parent / ".runtime"
    settings = dict(line.split("=", 1) for line in (runtime / "stack.env").read_text().splitlines())
    host = "pebble." + settings["WONTON_HOME_ZONE"] + ":14000"
    if "/" in args.email or "\\" in args.email or "@" not in args.email:
        parser.error("Invalid account email")
    account = args.home / "lego/accounts" / host.replace(":", "_") / args.email
    if (account / "account.json").exists():
        parser.error("Account exists; reuse it rather than silently rotating it")
    keys = account / "keys"
    keys.mkdir(parents=True, mode=0o700)
    key = keys / (args.email + ".key")
    if key.exists():
        parser.error("Account key exists; inspect interrupted registration before continuing")
    subprocess.run(["openssl", "genrsa", "-traditional", "-out", str(key), "2048"], check=True)
    key.chmod(0o600)
    directory = json.loads(curl(runtime, settings, "https://" + host + "/dir"))
    resource = register(runtime, settings, key, args.email, directory)
    output = account / "account.json"
    output.write_text(json.dumps(resource, indent=2) + "\n")
    output.chmod(0o600)
    print(resource["registration"]["uri"])


if __name__ == "__main__":
    main()
