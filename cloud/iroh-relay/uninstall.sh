#!/usr/bin/env bash
# 卸载 iroh 中继。默认保留 /etc/wonton-iroh-relay（配置、令牌、证书副本），--purge 连用户和数据一起删。
#   bash uninstall.sh
#   bash uninstall.sh --purge
set -euo pipefail

PREFIX=/opt/wonton-iroh-relay
ETC=/etc/wonton-iroh-relay
STATE=/var/lib/wonton-iroh-relay
USER=wonton-iroh-relay
DOMAIN=relay.808cbd.com
DEPLOY_HOOK=/etc/letsencrypt/renewal-hooks/deploy/wonton-iroh-relay.sh

die() {
    echo "uninstall: $*" >&2
    exit 1
}

[[ $EUID -eq 0 ]] || die "要用 root 跑"

systemctl disable --now wonton-iroh-relay.service 2>/dev/null || true
rm -f /etc/systemd/system/wonton-iroh-relay.service
systemctl daemon-reload

rm -f "/etc/nginx/sites-enabled/$DOMAIN" "/etc/nginx/sites-available/$DOMAIN"
if command -v nginx >/dev/null; then
    nginx -t && systemctl reload nginx
fi

rm -f "$DEPLOY_HOOK"
rm -rf "$PREFIX"

if [[ ${1-} == --purge ]]; then
    if command -v certbot >/dev/null && [[ -d /etc/letsencrypt/live/$DOMAIN ]]; then
        certbot delete --cert-name "$DOMAIN" --non-interactive || true
    fi
    rm -rf "$ETC" "$STATE"
    id "$USER" >/dev/null 2>&1 && userdel "$USER"
    echo "uninstall: 已删除用户、配置、数据和证书"
else
    echo "uninstall: 服务已删；配置和令牌保留在 $ETC，用户保留。彻底删用 --purge"
fi
