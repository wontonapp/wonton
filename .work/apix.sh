#!/bin/bash
# dev.sh 的 api 的替代：容器里连本机发布端口，用 --resolve 让 nip.io 解析到 127.0.0.1。
set -eu
H=100.83.35.101.nip.io P=19680 CORE=wonton-tt-linksv
TOK=$(docker exec "$CORE" cat /home/npchy/wonton-dev/tt/link-survey/deploy/dev/core/.data/core/owner-token)
m=$1; path=$2; shift 2
docker run --rm -i --network "container:$CORE" curlimages/curl:8.23.0 --noproxy '*' -sS --fail-with-body -X "$m" \
  --resolve "$H:$P:127.0.0.1" -H "Authorization: Bearer $TOK" "$@" "http://$H:$P$path"
