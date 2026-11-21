#!/bin/sh
# 首次启动前把下载器地址预置进 AB 配置。仅在配置文件还不存在且 torrent_client 能力地址
# 已注入时写：AB 只在 config.json 不存在时读 AB_* 环境变量，且写入后配置即不再等于出厂
# 默认、首次打开的设置向导会关闭——这里只写下载器一项，其余字段 AB 加载时按模型补默认值。
set -eu

conf=/app/config/config.json
if [ -f "$conf" ]; then
    echo "seed: 配置已存在，不动用户配置"
    exit 0
fi
if [ -z "${WONTON_TORRENT_CLIENT_URL:-}" ]; then
    echo "seed: torrent_client 能力地址未注入，跳过下载器预置（走应用的设置向导）"
    exit 0
fi

# host 接受完整 URL（qb_downloader.py 对含 :// 的值原样使用）；下载路径用 qBittorrent
# 插件的 /downloads 挂载点。账号口令留 AB 出厂默认：qBittorrent 插件对核心转发来的请求免凭据。
mkdir -p /app/config
cat >"$conf" <<EOF
{
    "plugins": {
        "instances": [
            {
                "id": "default",
                "point": "downloader",
                "provider": "qbittorrent",
                "options": {
                    "host": "${WONTON_TORRENT_CLIENT_URL}",
                    "path": "/downloads/Bangumi",
                    "ssl": false
                }
            }
        ]
    }
}
EOF
echo "seed: 已预置下载器 ${WONTON_TORRENT_CLIENT_URL}"
