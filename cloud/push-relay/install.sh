#!/usr/bin/env bash
# 在公网主机上装推送中继。可以重复跑：已有的配置、密钥和证书都不覆盖。
#
#   DOMAIN=push.example.com bash install.sh          # 必填；装服务 + vhost + 签该域证书
#   DOMAIN=push.example.com bash install.sh --no-cert # 只装服务，不碰证书
#   DOMAIN=push.example.com bash install.sh --offline-binary /tmp/gorush   # 用已下好的 gorush
#   DOMAIN=push.example.com bash install.sh --email ops@example.com        # 主机还没有 certbot 账号时
#
# 需要 root；宿主机本身不用 Docker。
set -euo pipefail

GORUSH_VERSION=1.22.0
GORUSH_SHA256=f772bca10515203fba468e696b9388a0c39d0a1183c27886722b6febc94b4745
GORUSH_URL="https://github.com/appleboy/gorush/releases/download/v${GORUSH_VERSION}/gorush-${GORUSH_VERSION}-linux-amd64"

PREFIX=/opt/wonton-push-relay
ETC=/etc/wonton-push-relay
STATE=/var/lib/wonton-push-relay
USER=wonton-push
# 域名没有默认值：不给 DOMAIN 就退出，避免把真实域漏进仓库。
DOMAIN=${DOMAIN:-}
# 这台机的主人域名（可选）：只用来把 certbot 装错的证书路径换回去。
OWNER_DOMAIN=${OWNER_DOMAIN:-}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CERT_DIR=

die() {
    echo "install: $*" >&2
    exit 1
}

# DOMAIN 必填；OWNER_DOMAIN 可选。CERT_DIR 由 DOMAIN 推出来。
require_domain() {
    [[ -n $DOMAIN ]] || die "缺 DOMAIN：DOMAIN=push.example.com $0"
    [[ $DOMAIN =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$ ]] ||
        die "DOMAIN=$DOMAIN 不是合法域名（只收小写字母、数字、点和短横线）"
    if [[ -n $OWNER_DOMAIN && ! $OWNER_DOMAIN =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$ ]]; then
        die "OWNER_DOMAIN=$OWNER_DOMAIN 不是合法域名"
    fi
    CERT_DIR=/etc/letsencrypt/live/$DOMAIN
}

# 把模板里的 @DOMAIN@ 换成 $DOMAIN，按给定权限装到目标路径（默认 0644）。
render_template() {
    local template=$1 target=$2 mode=${3:-0644}
    local tmp
    tmp=$(mktemp)
    sed "s|@DOMAIN@|$DOMAIN|g" "$template" >"$tmp"
    install -m "$mode" "$tmp" "$target"
    rm -f "$tmp"
}

need_root() {
    [[ $EUID -eq 0 ]] || die "要用 root 跑"
}

create_user() {
    if ! id "$USER" >/dev/null 2>&1; then
        useradd --system --home "$STATE" --shell /usr/sbin/nologin "$USER"
    fi
    install -d -m 0750 -o root -g "$USER" "$ETC" "$ETC/servers.d"
    install -d -m 0750 -o "$USER" -g "$USER" "$STATE" "$PREFIX"
}

install_files() {
    install -m 0644 "$HERE/relay.py" "$PREFIX/relay.py"
    install -m 0644 "$HERE/wonton-push-relay.service" /etc/systemd/system/wonton-push-relay.service
    install -m 0644 "$HERE/wonton-push-gorush.service" /etc/systemd/system/wonton-push-gorush.service
    if [[ ! -e "$ETC/relay.json" ]]; then
        render_template "$HERE/relay.json.template" "$ETC/relay.json"
    fi
    if [[ ! -e "$ETC/gorush.yml" ]]; then
        install -m 0644 "$HERE/gorush.yml" "$ETC/gorush.yml"
    fi
}

install_gorush() {
    local source=$1
    if [[ -n $source ]]; then
        install -m 0755 "$source" "$PREFIX/gorush"
    elif [[ -x $PREFIX/gorush ]] && "$PREFIX/gorush" --version | grep -q "GoRush $GORUSH_VERSION"; then
        echo "install: 已有 gorush $GORUSH_VERSION，跳过下载"
    else
        local tmp
        tmp=$(mktemp)
        curl -fsSL --retry 3 --max-time 300 -o "$tmp" "$GORUSH_URL"
        echo "$GORUSH_SHA256  $tmp" | sha256sum -c - >/dev/null
        install -m 0755 "$tmp" "$PREFIX/gorush"
        rm -f "$tmp"
    fi
    "$PREFIX/gorush" --version
}

# 没有 APNs 密钥时用一把本地生成的 P-256 假钥匙：gorush 起得来，发到 APNs 会 403 InvalidProviderToken。
ensure_fake_key() {
    if [[ -e "$ETC/apns.p8" ]]; then
        return
    fi
    # Apple 的 .p8 是 PKCS#8；gorush 按 PKCS#8 解，SEC1（ecparam 默认）会起不来。
    openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:prime256v1 -out "$ETC/apns.p8"
    chown root:"$USER" "$ETC/apns.p8"
    chmod 0640 "$ETC/apns.p8"
    touch "$ETC/apns.p8.is-fake"
    echo "install: 用的是假 APNs 密钥，真推送要换成主人的 .p8（见 README）"
}

install_vhost() {
    local target=/etc/nginx/sites-available/$DOMAIN
    local source=$HERE/nginx/push.bootstrap.conf.template
    [[ -d $CERT_DIR ]] && source=$HERE/nginx/push.conf.template
    render_template "$source" "$target"
    ln -sfn "$target" "/etc/nginx/sites-enabled/$DOMAIN"
    nginx -t
    systemctl reload nginx
}

# 没有服务账号时只说一句：gorush 默认关着 Android，APNs 不受影响。
fcm_hint() {
    [[ -e "$ETC/fcm-service-account.json" ]] && return
    echo "install: 没有 FCM 服务账号，gorush 的 Android 保持关闭（见 README 的“FCM 配置”）"
}

# certbot 会把推送证书装到主人的通配符块上，换回主人自己的证书。没有 OWNER_DOMAIN 时跳过。
restore_owner_cert() {
    [[ -n $OWNER_DOMAIN ]] || { echo "install: 未设 OWNER_DOMAIN，certbot 若改过主人块不会恢复" >&2; return 0; }
    local owner=/etc/nginx/sites-available/$OWNER_DOMAIN
    [[ -f $owner ]] || return 0
    sed -i "s#/etc/letsencrypt/live/$DOMAIN/#/etc/letsencrypt/live/$OWNER_DOMAIN/#g" "$owner"
}

finalize_vhost() {
    render_template "$HERE/nginx/push.conf.template" "/etc/nginx/sites-available/$DOMAIN"
    nginx -t
    systemctl reload nginx
}

wait_healthy() {
    for _ in $(seq 1 20); do
        if curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:8790/healthz; then
            return 0
        fi
        sleep 0.5
    done
    die "服务起来了但 /healthz 不通，看 journalctl -u wonton-push-relay"
}

main() {
    local offline_binary=""
    local email=""
    local cert=1
    while (($#)); do
        case $1 in
            --offline-binary)
                [[ -n ${2-} ]] || die "--offline-binary 要给文件"
                offline_binary=$2
                shift 2
                ;;
            --email)
                [[ -n ${2-} ]] || die "--email 要给地址"
                email=$2
                shift 2
                ;;
            --no-cert)
                cert=0
                shift
                ;;
            *)
                die "不认识的参数 $1"
                ;;
        esac
    done

    require_domain

    need_root
    command -v openssl >/dev/null || die "缺 openssl"
    command -v curl >/dev/null || die "缺 curl"
    python3 -c 'import cryptography' 2>/dev/null || die "缺 python3 的 cryptography（apt install python3-cryptography）"

    create_user
    install_files
    install_gorush "$offline_binary"
    ensure_fake_key
    fcm_hint
    install_vhost

    systemctl daemon-reload
    systemctl enable --now wonton-push-gorush.service wonton-push-relay.service
    systemctl restart wonton-push-gorush.service wonton-push-relay.service
    wait_healthy

    if ((cert)); then
        if [[ -d $CERT_DIR ]]; then
            echo "install: 证书已存在，跳过签发"
        else
            local -a account=()
            [[ -n $email ]] && account=(--email "$email")
            certbot --nginx -d "$DOMAIN" --redirect --non-interactive --agree-tos "${account[@]}"
        fi
        restore_owner_cert
        finalize_vhost
    fi

    echo "install: 完成"
    curl -sS -o /dev/null -w "本机 /healthz %{http_code}\n" http://127.0.0.1:8790/healthz
    curl -sS -o /dev/null -w "公网 https 未带鉴权 %{http_code}（应为 401）\n" -X POST -H "Content-Encoding: aes128gcm" --data-binary x "https://$DOMAIN/push/$(printf 'ab%.0s' {1..32})" || true
    echo "install: 把家里服务器的 VAPID 公钥写进 $ETC/servers.d/<名字>.json 后才能真发"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
