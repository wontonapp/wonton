#!/usr/bin/env bash
# Warn when a UniFFI action enum approaches the 32 KiB metadata buffer.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

if (($# == 0)); then
  cargo build -p wonton-protocol --features uniffi
  libraries=("${CARGO_TARGET_DIR:-target}/debug/libwonton_protocol.rlib")
else
  libraries=("$@")
fi

for library in "${libraries[@]}"; do
  if [[ ! -f "$library" ]]; then
    echo "找不到 $library；先运行 cargo build -p wonton-protocol --features uniffi" >&2
    exit 1
  fi
done

found=0
for library in "${libraries[@]}"; do
  while read -r size symbol; do
    found=1
    bytes=$((16#$size))
    printf '%s %d 字节\n' "$symbol" "$bytes"
    if ((bytes > 16384)); then
      printf '警告：%s 的 UniFFI 元数据为 %d 字节，超过 16 KiB 预警线\n' "$symbol" "$bytes" >&2
    fi
  done < <(
    nm -S --defined-only "$library" 2>/dev/null |
      awk '$NF ~ /^UNIFFI_META_WONTON_PROTOCOL_ENUM_.*ACTION$/ { print $2, $NF }'
  )
done

if ((found == 0)); then
  echo "找不到 UNIFFI_META_WONTON_PROTOCOL_ENUM_*ACTION 符号；构建可能没有启用 UniFFI" >&2
  exit 1
fi
