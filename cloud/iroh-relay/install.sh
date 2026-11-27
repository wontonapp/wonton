#!/usr/bin/env bash
# 在公网 Ubuntu 主机上装 iroh 中继：TCP 443 由 nginx 终结 TLS 后反代，中继只监听回环，
# UDP 7842 的 QUIC 地址发现直接对公网（安全组要放行 7842/udp，下面启动后会自检）。按 cloud/iroh-relay/README.md 的部署一节使用。
# 本文件里的 808cbd.com 是真实部署域；换域改 DOMAIN/OWNER_DOMAIN。
#
# 可以重复跑：已有的 config.toml、令牌、证书都不覆盖；同版本二进制不重下。
#   bash install.sh                                  # 装服务 + vhost + 签 relay.808cbd.com 的证书
#   bash install.sh --no-cert                        # 只装服务，不签证书
#   bash install.sh --offline-binary /tmp/iroh-relay # 用已经下好的官方二进制（会校验版本）
#   bash install.sh --email ops@example.com          # 主机上还没有 certbot 账号时
#
# 需要 root；宿主机不用 Docker。上游版本和 sha256 在 release.env。
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=release.env
. "$HERE/release.env"

PREFIX=/opt/wonton-iroh-relay
ETC=/etc/wonton-iroh-relay
STATE=/var/lib/wonton-iroh-relay
USER=wonton-iroh-relay
DOMAIN=relay.808cbd.com
OWNER_DOMAIN=808cbd.com
CERT_DIR=/etc/letsencrypt/live/$DOMAIN
DEPLOY_HOOK=/etc/letsencrypt/renewal-hooks/deploy/wonton-iroh-relay.sh

die() {
    echo "install: $*" >&2
    exit 1
}

need_root() {
    [[ $EUID -eq 0 ]] || die "要用 root 跑"
}

create_user() {
    if ! id "$USER" >/dev/null 2>&1; then
        useradd --system --home "$STATE" --shell /usr/sbin/nologin "$USER"
    fi
    install -d -m 0750 -o root -g "$USER" "$ETC" "$ETC/tls"
    install -d -m 0750 -o "$USER" -g "$USER" "$STATE" "$PREFIX"
    # install -d 不改已存在的目录：重跑时把被人改宽的权限收回来。
    chmod 0750 "$ETC" "$ETC/tls"
}

# 只支持 upstream 发了官方 asset 的架构；sha256 在 release.env。
download_binary() {
    local arch sha url
    arch=$(uname -m)
    case $arch in
        x86_64) sha=$IROH_RELAY_SHA256_X86_64 ;;
        aarch64) sha=$IROH_RELAY_SHA256_AARCH64 ;;
        *) die "上游没有 $arch 的官方 release" ;;
    esac
    url="$IROH_RELAY_RELEASE_BASE/iroh-relay-v$IROH_RELAY_VERSION-$arch-unknown-linux-musl.tar.gz"
    curl -fsSL --retry 3 --max-time 300 -o "$WORK/relay.tar.gz" "$url"
    echo "$sha  $WORK/relay.tar.gz" | sha256sum -c - >/dev/null || die "sha256 不匹配"
    tar xzf "$WORK/relay.tar.gz" -C "$WORK" ./iroh-relay
    install -m 0755 "$WORK/iroh-relay" "$PREFIX/iroh-relay.new"
}

install_binary() {
    local offline=${1-}
    if [[ -n $offline ]]; then
        [[ -x $offline ]] || die "--offline-binary 指向的文件不可执行"
        "$offline" --version | grep -qx "iroh-relay $IROH_RELAY_VERSION" ||
            die "离线二进制的版本不是 $IROH_RELAY_VERSION"
        install -m 0755 "$offline" "$PREFIX/iroh-relay.new"
    elif [[ -x $PREFIX/iroh-relay ]] &&
        "$PREFIX/iroh-relay" --version 2>/dev/null | grep -qx "iroh-relay $IROH_RELAY_VERSION"; then
        echo "install: 已有 iroh-relay $IROH_RELAY_VERSION，跳过下载"
        return
    else
        download_binary
    fi
    # 运行中的进程占着旧 inode，rename 过去不影响它；重启在最后统一做。
    mv -f "$PREFIX/iroh-relay.new" "$PREFIX/iroh-relay"
    "$PREFIX/iroh-relay" --version
}

install_files() {
    install -m 0644 "$HERE/wonton-iroh-relay.service" /etc/systemd/system/wonton-iroh-relay.service
    install -d -m 0755 "$(dirname "$DEPLOY_HOOK")"
    install -m 0755 "$HERE/nginx/cert-deploy-hook.sh" "$DEPLOY_HOOK"
}

# 配置和令牌只在缺失时生成，重跑不覆盖主人的改动。
ensure_config() {
    if [[ ! -e $ETC/config.toml ]]; then
        install -m 0640 -o root -g "$USER" "$HERE/config.toml.example" "$ETC/config.toml"
    fi
    if [[ ! -e $ETC/relay.env ]]; then
        # 没有令牌时上游拒绝启动（失败即停）；0600 只在 root 和 systemd 之间。
        printf 'IROH_RELAY_ACCESS_TOKEN=%s\n' "$(openssl rand -hex 32)" >"$ETC/relay.env.tmp"
        chown root:root "$ETC/relay.env.tmp"
        chmod 0600 "$ETC/relay.env.tmp"
        mv "$ETC/relay.env.tmp" "$ETC/relay.env"
        echo "install: 已生成中继准入令牌 $ETC/relay.env（0600）。改它后 systemctl restart wonton-iroh-relay"
    fi
    # 同上：重跑时收回已存在配置的权限，令牌始终只有 root/systemd 读。
    chmod 0640 "$ETC/config.toml"
    chmod 0600 "$ETC/relay.env"
}

install_vhost() {
    local source=$1
    local target=/etc/nginx/sites-available/$DOMAIN
    install -m 0644 "$source" "$target"
    ln -sfn "$target" "/etc/nginx/sites-enabled/$DOMAIN"
    nginx -t
    systemctl reload nginx
}

# certbot --nginx 按 server_name 找块时会被主人的 *.808cbd.com 骗到，把证书装到 808cbd.com 块上；
# 装完把主人块的证书路径换回去。
restore_owner_cert() {
    local owner=/etc/nginx/sites-available/$OWNER_DOMAIN
    if [[ ! -f $owner ]]; then
        echo "install: 警告：找不到 $owner，没把主人的证书路径换回去；nginx -T 里核对一下" >&2
        return 0
    fi
    sed -i "s#/etc/letsencrypt/live/$DOMAIN/#/etc/letsencrypt/live/$OWNER_DOMAIN/#g" "$owner"
}

ensure_cert() {
    local email=${1-}
    if [[ -d $CERT_DIR ]]; then
        echo "install: 证书已存在，跳过签发"
    else
        command -v certbot >/dev/null || die "缺 certbot（apt install certbot python3-certbot-nginx）"
        install_vhost "$HERE/nginx/relay.bootstrap.conf"
        local -a account=()
        [[ -n $email ]] && account=(--email "$email")
        certbot --nginx -d "$DOMAIN" --redirect --non-interactive --agree-tos "${account[@]}"
        restore_owner_cert
    fi
    # 复制给非 root 的中继进程读；首次和续期都走同一个 hook。
    sh "$DEPLOY_HOOK"
}

# 判定准入模式：上游 access 默认 Everyone；只识别“显式 everyone”和“非 everyone 的 access 配置”两类。
# 非 everyone 包括 [access]、[access.http]、[access.allowlist] 等表变体和行内表 access = { ... }，一律 restricted。
admission_mode() {
    local cfg=$1
    if grep -Eq '^[[:space:]]*access[[:space:]]*=[[:space:]]*["'\'']?everyone["'\'']?' "$cfg"; then
        echo everyone
    elif grep -Eq '^[[:space:]]*\[access(\.|\])' "$cfg"; then
        echo restricted
    elif grep -Eq '^[[:space:]]*access[[:space:]]*=' "$cfg"; then
        echo restricted
    else
        echo everyone-default
    fi
}

# 启动后自检：QAD 必须在所有网卡上监听（quic_bind_addr 放错节时上游静默忽略、只绑回环）。
post_start_check() {
    local qad
    command -v ss >/dev/null || die "缺 ss（iproute2），装完没法自检 QAD"
    qad=$(ss -lunH "sport = :7842" || true)
    echo "$qad" | grep -qE '(\*|0\.0\.0\.0|\[::\]):7842' ||
        die "UDP 7842 没有对外监听（QAD 需要）：$qad；检查 [tls] 节的 quic_bind_addr 和安全组"
    echo "install: QAD 对外监听正常：$qad"
    case $(admission_mode "$ETC/config.toml") in
        everyone)
            echo "install: 准入 = everyone（公开，谁都能用）"
            ;;
        restricted)
            echo "install: 准入 = [access] 节（shared_token/名单/http 回调；非 everyone；令牌在 $ETC/relay.env，只发自己的设备）"
            ;;
        everyone-default)
            echo "install: 准入 = everyone（上游默认：配置里没有 [access] 节；推荐加回 [access] shared_token = [] 变成默认拒绝）"
            ;;
    esac
}

wait_healthy() {
    for _ in $(seq 1 20); do
        if curl --noproxy '*' -fsS -o /dev/null --max-time 2 "http://127.0.0.1:8742/healthz"; then
            return 0
        fi
        sleep 0.5
    done
    die "服务起来了但 /healthz 不通，看 journalctl -u wonton-iroh-relay"
}

main() {
    local offline=""
    local email=""
    local cert=1
    while (($#)); do
        case $1 in
            --offline-binary)
                [[ -n ${2-} ]] || die "--offline-binary 要给文件"
                offline=$2
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

    need_root
    WORK=$(mktemp -d)
    trap 'rm -rf "$WORK"' EXIT
    command -v openssl >/dev/null || die "缺 openssl"
    command -v curl >/dev/null || die "缺 curl"
    command -v nginx >/dev/null || die "缺 nginx（apt install nginx）"

    create_user
    install_files
    install_binary "$offline"
    ensure_config

    systemctl daemon-reload
    systemctl enable wonton-iroh-relay.service

    if ((cert)); then
        ensure_cert "$email"
        install_vhost "$HERE/nginx/relay.808cbd.com.conf"
        systemctl restart wonton-iroh-relay.service
        wait_healthy
        post_start_check
        curl --noproxy '*' -fsS --resolve "$DOMAIN:443:127.0.0.1" -o /dev/null -w "公网 vhost /healthz %{http_code}\n" "https://$DOMAIN/healthz"
    elif [[ -d $CERT_DIR ]]; then
        install_vhost "$HERE/nginx/relay.808cbd.com.conf"
        systemctl restart wonton-iroh-relay.service
        wait_healthy
        post_start_check
    else
        install_vhost "$HERE/nginx/relay.bootstrap.conf"
        if systemctl restart wonton-iroh-relay.service; then
            post_start_check
        else
            echo "install: 还没证书，服务起不来；签好 $DOMAIN 的证书后 install.sh 重跑"
        fi
    fi

    echo "install: 完成（内测期准入是 shared_token：只把连接信息发给自己的设备，不公开注册）"
    echo "install: 令牌在 $ETC/relay.env（0600），日志：journalctl -u wonton-iroh-relay，指标：curl --noproxy '*' http://127.0.0.1:19090/metrics"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
