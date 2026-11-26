#!/usr/bin/env bash
# 回声 Bot（matrix.bot）：核心代它登录、建好私聊，手机经隧道和它收发。
# 从 smoke.sh 拆出来（文件行数门禁），source 进来共用同一批变量和助手（smoke-cleanup.sh 同款）。
if step echo "回声 Bot（matrix.bot）：核心代它登录、建好私聊，手机经隧道和它收发"; then
build --target echo-bot --build-arg WONTON_BUILD_COMMIT -t "$echo_image" -f "$here/Dockerfile" "$repo"
install_echo() {
  local installed
  installed="$("$dev" install "$repo/plugins/examples/echo-bot" --yes 2>/dev/null)" ||
    fail "回声 Bot 安装失败：$installed"
  expect "回声 Bot 安装完成，同步上了 Matrix" \
    "$(json status.state <<<"$installed") $(json status.health <<<"$installed")" "running healthy"
}
# 核心注入 Bot 容器的变量。
bot_env() { docker inspect "$echo_bot" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
install_echo
bot_user="$(bot_env WONTON_MATRIX_USER_ID)"
bot_token="$(bot_env WONTON_MATRIX_ACCESS_TOKEN)"
# Tuwunel 的库是 volume、跨栈留着，前几轮的 @echo 可能还在，核心按 bot.rs 的 reserve_bot_account 加 -2 后缀。
[[ "$bot_user" == @echo*"$WONTON_DEV_DOMAIN" ]] || fail "Bot 的账号不对：$bot_user"
ok "Bot 有自己的账号：$bot_user"
expect "Bot 经核心在插件网络里的别名访问 Matrix" \
  "$(bot_env WONTON_MATRIX_HOMESERVER)" "http://matrix.wonton.internal:$WONTON_DEV_PORT"
expect "Bot 的显示名" "$(owner_api "/profile/$bot_user/displayname" | json displayname)" 回声
home_room="$(direct_rooms "$bot_user")"
[[ "$home_room" == '!'* && "$home_room" != *$'\n'* ]] || fail "主人的 m.direct 里应有一个和 Bot 的私聊：$home_room"
ok "主房间记在主人的 m.direct 里"
expect "主人和 Bot 都已经在主房间里" \
  "$(owner_api "/rooms/$home_room/joined_members" | python3 -c 'import json, sys; print(" ".join(sorted(json.load(sys.stdin)["joined"])))')" \
  "$bot_user @owner:$WONTON_DEV_DOMAIN"
expect "主房间不加密" \
  "$(owner_api "/rooms/$home_room/state" | python3 -c 'import json, sys; print(any(e["type"] == "m.room.encryption" for e in json.load(sys.stdin)))')" False
# 主人的一台新设备（echo-bot probe）经手机的本地代理登录，发一条消息，等 Bot 的回声。
jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
probe="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
  --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$home_room" \
  --bot "$bot_user" 2>&1)" || fail "收发失败：$probe"
ok "经隧道发出消息，收到 Bot 的回声"
# `/delay` 到点才回：记时间，早回就是没延迟。
jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
started="$(date +%s)"
delay_probe="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
  --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$home_room" --bot "$bot_user" \
  --reply any --match "回声：等一会儿" --text "/delay 5 等一会儿" --wait 30 2>&1)" ||
  fail "延迟回声失败：$delay_probe"
elapsed=$(( $(date +%s) - started ))
((elapsed >= 5)) || fail "回声没有延迟，${elapsed} 秒就回了"
ok "/delay 5 的回声 ${elapsed} 秒后才到"
# crypto store 丢了（数据卷被删、容器重建没挂卷）：房间不加密，Bot 换一台新设备后照常收发。
docker exec "$echo_bot" sh -c 'rm -rf /data/*'
docker restart "$echo_bot"
for _ in $(seq 60); do
  [[ "$(docker inspect "$echo_bot" --format '{{.State.Health.Status}}' 2>/dev/null)" == healthy ]] && break
  sleep 1
done
expect "删掉 crypto store 重启后 Bot 自己恢复健康" \
  "$(docker inspect "$echo_bot" --format '{{.State.Health.Status}}')" healthy
jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
lost_store_probe="$(docker run --rm --network "container:$core" "$echo_image" probe \
  --homeserver "$matrix_url" --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" \
  --room "$home_room" --bot "$bot_user" --wait 30 2>&1)" ||
  fail "crypto store 丢了重启后 Bot 不能回声：$lost_store_probe"
ok "crypto store 丢了重启后 Bot 照常回声"
report="$("$dev" uninstall agent.echo)"
expect "卸载时吊销 Bot 的设备" "$(json bots.0.devices_removed <<<"$report") $(json bots.0.deactivated <<<"$report")" "1 false"
expect "Bot 的令牌立即失效" \
  "$(t -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $bot_token" "$matrix_url/_matrix/client/v3/account/whoami")" 401
install_echo
expect "不删数据重装：同一个账号、同一个主房间" \
  "$(bot_env WONTON_MATRIX_USER_ID) $(direct_rooms "$bot_user")" "$bot_user $home_room"
jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
probe="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
  --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$home_room" \
  --bot "$bot_user" 2>&1)" || fail "重装换新设备后收发没有恢复：$probe"
ok "重装换新设备后恢复收发"
report="$("$dev" uninstall agent.echo --purge)"
expect "删除数据时停用 Bot 账号" "$(json bots.0.deactivated <<<"$report")" true
members="$(owner_api "/rooms/$home_room/joined_members")"
expect "停用的 Bot 退出了主房间" \
  "$(python3 -c 'import json, sys; print(sys.argv[1] in json.load(sys.stdin)["joined"])' "$bot_user" <<<"$members")" False
fi

