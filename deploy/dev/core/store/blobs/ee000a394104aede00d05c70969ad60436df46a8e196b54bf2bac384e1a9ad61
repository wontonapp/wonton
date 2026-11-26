#!/bin/sh
# 给核心的 MCP 总线准备 AutoBangumi 的 MCP 令牌。AB 的 /mcp 只认 scope=mcp 的令牌
# （mcp_whitelist 默认拒绝一切 IP 访问），令牌由应用签发、存在应用数据库里，
# 容器重建后仍有效：已有有效令牌就复用；失效才重新登录签发。
# 登录用出厂默认账号（应用没有改初始密码的配置项）：用户改过密码且旧令牌已失效时，
# 这里如实失败，错误写明去应用里处理后重装。
set -eu
umask 077

base=http://autobangumi:7892
token_file=/bridge/token
jar=/tmp/ab-session

api_code() {
    curl --noproxy '*' -sS -o /dev/null -w '%{http_code}' "$@"
}

# 令牌验证只能打 /mcp 本身：普通接口的 Bearer 只认 scope=api 的令牌，
# scope=mcp 的令牌只有 MCP 入口认。403 是访问被拒（令牌无效），其余状态算有效。
if [ -s "$token_file" ]; then
    status=$(api_code -X POST -H "Authorization: Bearer $(cat "$token_file")" \
        -H 'Content-Type: application/json' --data '{}' "$base/mcp") ||
        { echo "mcp-token: 检查已有令牌时连不上 AutoBangumi"; exit 1; }
    if [ "$status" != 403 ]; then
        echo "mcp-token: 已有令牌有效，复用"
        exit 0
    fi
    echo "mcp-token: 已有令牌被拒绝（HTTP $status），重新签发"
fi

rm -f "$jar"
status=$(api_code -c "$jar" -X POST -H 'Content-Type: application/x-www-form-urlencoded' \
    --data 'username=admin&password=adminadmin' "$base/api/v1/auth/login") ||
    { echo "mcp-token: 登录 AutoBangumi 失败（传输错误）"; exit 1; }
if [ "$status" != 200 ]; then
    echo "mcp-token: 登录 AutoBangumi 失败（HTTP $status）：admin 账号密码已不是出厂值，且旧 MCP 令牌已失效。请在应用里改回或处理令牌后重装插件。"
    exit 1
fi

created=$(curl --noproxy '*' -fsS -b "$jar" -X POST -H 'Content-Type: application/json' \
    --data '{"name":"wonton-mcp","scope":"mcp"}' "$base/api/v1/tokens") ||
    { echo "mcp-token: 签发 MCP 令牌失败"; exit 1; }
token=$(printf '%s' "$created" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
[ -n "$token" ] || { echo "mcp-token: 签发 MCP 令牌的响应里没有令牌"; exit 1; }

printf '%s' "$token" >"$token_file"
echo "mcp-token: 已签发 MCP 令牌"
