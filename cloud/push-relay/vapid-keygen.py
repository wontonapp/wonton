#!/usr/bin/env python3
"""给家里服务器生成 VAPID 密钥对。

私钥是不带填充的 base64url（32 字节 P-256 标量），写进给的路径并设 0600；公钥打在标准输出，拿去在
中继上登记（`server-add.sh <名字> <公钥>`）。核心用 `WONTON_PUSH_VAPID_KEY_FILE` 指向这个文件。

    python3 vapid-keygen.py /etc/wonton/push-vapid.key
"""

import base64
import os
import sys
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ec


def b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def main() -> int:
    if len(sys.argv) != 2:
        print(f"用法：{sys.argv[0]} <私钥文件>", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    if path.exists():
        print(f"{path} 已存在，不覆盖", file=sys.stderr)
        return 1
    key = ec.generate_private_key(ec.SECP256R1())
    scalar = key.private_numbers().private_value.to_bytes(32, "big")
    public = key.public_key().public_bytes(
        serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(b64(scalar), encoding="utf-8")
    os.chmod(path, 0o600)
    print(b64(public))
    return 0


if __name__ == "__main__":
    sys.exit(main())
