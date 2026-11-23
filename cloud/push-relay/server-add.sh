#!/usr/bin/env bash
# 在中继上登记一台家里服务器：登记 VAPID 公钥和它自己的限速，然后让服务重读登记表。
#
#   bash server-add.sh <名字> <公钥> [每分钟条数] [突发条数]
#
# 公钥由家里服务器上的 vapid-keygen.py 生成。重复用同一个名字会覆盖（换钥匙、改限速都走这里）。
set -euo pipefail

ETC=/etc/wonton-push-relay
USER=wonton-push

die() {
    echo "server-add: $*" >&2
    exit 1
}

[[ $EUID -eq 0 ]] || die "要用 root 跑"
[[ $# -ge 2 && $# -le 4 ]] || die "用法：$0 <名字> <公钥> [每分钟条数] [突发条数]"
name=$1
public_key=$2
rate=${3:-120}
burst=${4:-60}
[[ $name =~ ^[a-z0-9][a-z0-9-]{0,40}$ ]] || die "名字只能是小写字母、数字和短横线"

python3 - "$name" "$public_key" "$rate" "$burst" "$ETC/servers.d/$name.json" <<'EOF'
import base64, json, sys

name, public_key, rate, burst, path = sys.argv[1:]
raw = base64.urlsafe_b64decode(public_key + "=" * (-len(public_key) % 4))
if len(raw) != 65 or raw[0] != 4:
    sys.exit(f"server-add: {name} 的公钥不是未压缩的 P-256 点")
if int(rate) <= 0 or int(burst) < 1:
    sys.exit("server-add: 限速要大于 0，突发至少为 1")
with open(path, "w", encoding="utf-8") as out:
    json.dump({"name": name, "public_key": public_key, "rate_per_minute": int(rate), "burst": int(burst)},
              out, ensure_ascii=False, indent=2)
    out.write("\n")
EOF

chown root:"$USER" "$ETC/servers.d/$name.json"
chmod 0640 "$ETC/servers.d/$name.json"
systemctl reload wonton-push-relay.service
echo "server-add: 已登记 $name（$rate/分，突发 $burst）"
