#!/usr/bin/env bash
# 生成 entry_certificate/lego_providers.txt（固定版本 lego 的 DNS provider 代码清单），再核对
# data/dns-providers.json 的预设：每家都得是这个版本里带 toml 的 provider，凭据变量是它 toml 里 [Configuration.Credentials] 的键。
# 只读网络，CI 不跑。版本从 entry_certificate.rs 的 LEGO_IMAGE 读，不另传，免得清单和镜像对不上；
# 升 LEGO_IMAGE 之后跑一次，清单和预设一起提交。要 GNU tar（macOS 用 gtar）。
#
#   scripts/gen-dns-presets.sh
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

source=crates/wonton-core/src/entry_certificate.rs
presets=crates/wonton-core/data/dns-providers.json
list=crates/wonton-core/src/entry_certificate/lego_providers.txt

image=$(grep -oE '"goacme/lego:v[^"]+"' "$source" | tr -d '"')
tag=$(sed -E 's/^[^:]+:(v[0-9.]+).*/\1/' <<<"$image")
[[ $tag =~ ^v[0-9.]+$ ]] || { echo "$source 里的 LEGO_IMAGE 读不出 tag：$image" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/lego.tar.gz" "https://codeload.github.com/go-acme/lego/tar.gz/refs/tags/$tag"
tar -xzf "$tmp/lego.tar.gz" -C "$tmp" --wildcards '*/providers/dns/*/*.toml'
dns=$(echo "$tmp"/lego-*/providers/dns)

# 只算带自己 toml 的目录：lego 里 provider 代码就是目录名，`--dns <代码>`。
{
  echo "# go-acme/lego $tag 的 providers/dns 目录，scripts/gen-dns-presets.sh 生成。"
  echo "# 不手改：和 entry_certificate.rs 的 LEGO_IMAGE 一起升，一起提交。"
  for toml in "$dns"/*/*.toml; do basename "$(dirname "$toml")"; done | sort
} >"$list"
echo "写 $list：$(grep -vc '^#' "$list") 家（lego $tag）"

failed=0
while read -r code vars; do
  toml="$dns/$code/$code.toml"
  if [[ ! -f $toml ]]; then
    echo "预设 $code 不在 lego $tag 里（没有 providers/dns/$code/$code.toml）" >&2
    failed=1
    continue
  fi
  credentials=$(awk '/^[[:space:]]*\[Configuration\.Credentials\]/ { inside = 1; next } /^[[:space:]]*\[/ { inside = 0 } inside && $2 == "=" { print $1 }' "$toml")
  for var in $vars; do
    if ! grep -qx "$var" <<<"$credentials"; then
      echo "预设 $code 的凭据 $var 不在 $tag 的 toml 必填项里；有的话改成上游的名字，没有就换一家" >&2
      failed=1
    fi
  done
done < <(python3 -c '
import json, sys
for item in json.load(open(sys.argv[1])):
    print(item["code"], " ".join(item["credential_vars"]))
' "$presets")

exit $failed
