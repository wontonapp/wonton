#!/bin/sh
# certbot deploy hook：续期成功后把证书复制给 iroh-relay 读（中继只读根、非 root），reload nginx。
# 中继自己每 24h 重读这两个文件，不用重启；nginx 要 reload 才会用新证书。
set -eu

DOMAIN=relay.808cbd.com
DEST=/etc/wonton-iroh-relay/tls
SRC=/etc/letsencrypt/live/$DOMAIN

install -d -m 0750 -o root -g wonton-iroh-relay "$DEST"
install -m 0640 -o root -g wonton-iroh-relay "$SRC/fullchain.pem" "$DEST/fullchain.pem"
install -m 0640 -o root -g wonton-iroh-relay "$SRC/privkey.pem" "$DEST/privkey.pem"
nginx -t
systemctl reload nginx
