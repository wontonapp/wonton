#!/bin/sh
set -eu

config_dir=/config/qBittorrent/config
config_file="$config_dir/qBittorrent.conf"

if [ ! -e "$config_file" ]; then
    mkdir -p "$config_dir"
    sed "s/__WONTON_CORE_ADDRESS__/${WONTON_CORE_ADDRESS:?}/g" /seed/qBittorrent.conf >"$config_file"
else
    # 重装会重建插件网络，核心地址可能变化；只刷新免认证白名单，保留 qBittorrent 其他配置。
    sed -i "s|^WebUI\\\\AuthSubnetWhitelist=.*|WebUI\\\\AuthSubnetWhitelist=${WONTON_CORE_ADDRESS:?}/32|" "$config_file"
    # 升级前的老配置没有 VueTorrent 两项；键已存在（用户可能关掉了备用 WebUI）就不碰。
    if ! grep -q '^WebUI\\AlternativeUIEnabled=' "$config_file"; then
        sed -i '/^WebUI\\AuthSubnetWhitelistEnabled=/a WebUI\\RootFolder=/vuetorrent' "$config_file"
        sed -i '/^WebUI\\AuthSubnetWhitelistEnabled=/a WebUI\\AlternativeUIEnabled=true' "$config_file"
    fi
fi

# 下载完成的通知邮件经核心的内部邮箱发信：不用认证、不加密，收件人是主人的内部邮箱，信转进主人的 Wonton 私聊。
# 键已存在就不碰：装好时种过、用户改过收件人或关了开关，重装都不覆盖。
if ! grep -q '^MailNotification\\enabled=' "$config_file"; then
    sed -i "s|^WebUI\\\\AuthSubnetWhitelistEnabled=.*|&\\nMailNotification\\\\enabled=true\\nMailNotification\\\\email=${WONTON_OWNER_EMAIL:?}\\nMailNotification\\\\sender=${WONTON_MAIL_FROM:?}\\nMailNotification\\\\smtp_server=${WONTON_SMTP_HOST:?}:${WONTON_SMTP_PORT:?}\\nMailNotification\\\\req_ssl=false\\nMailNotification\\\\req_auth=false|" "$config_file"
fi
grep -q '^MailNotification\\enabled=' "$config_file" ||
    { echo "qBittorrent.conf 里没有 WebUI\\AuthSubnetWhitelistEnabled，通知邮件写不进去；配置可能被改过，先恢复这项再重装"; exit 1; }

# 备用 WebUI 用 VueTorrent：官方发版取来解压进数据目录。版本与校验和写死在 docker-compose.yml。
version="${VUETORRENT_VERSION:?}"
sha256="${VUETORRENT_SHA256:?}"
dest=/vuetorrent
if [ "$(cat "$dest/.version" 2>/dev/null || true)" = "$version" ] && [ -f "$dest/public/index.html" ]; then
    exit 0
fi

zip=/tmp/vuetorrent.zip
url="https://github.com/VueTorrent/VueTorrent/releases/download/${version}/vuetorrent.zip"
# 经出网代理到 GitHub 发版 CDN 的链路可能停摆也可能只是慢（慢链路实测约 20-37 KB/s，6.5 MB 要 3-5 分钟）：
# 30 秒没等到 1 KiB 当瞬时错误触发重试，--max-time 给慢而不断的传输留足时间；
# 重试窗口要大于单次超时，被截断的传输才能靠 -C - 续传接着跑完。
if ! curl -fsSL --retry 4 --retry-delay 2 --retry-max-time 300 --connect-timeout 10 \
    --speed-limit 1024 --speed-time 30 --max-time 240 -C - -o "$zip" "$url"; then
    echo "下载 VueTorrent 失败：${url}（检查出网代理是否可用、GitHub 是否可达）"
    exit 1
fi
echo "${sha256}  $zip" | sha256sum -c - || { echo "VueTorrent 校验和不匹配，删除残留后退出"; rm -f "$zip"; exit 1; }
stage=/tmp/vuetorrent-extract
rm -rf "$stage"
mkdir -p "$stage"
unzip -q "$zip" -d "$stage"
[ -f "$stage/vuetorrent/public/index.html" ] ||
    { echo "vuetorrent.zip 里没有 vuetorrent/public/index.html"; exit 1; }
# 换版本时清掉旧文件再放进新文件；挂载点本身不删。
find "$dest" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
mv "$stage/vuetorrent/"* "$dest/"
rm -rf "$stage"
rm -f "$zip"
printf '%s\n' "$version" >"$dest/.version"
