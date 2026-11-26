#!/bin/sh
# 网页（3430）的入口：先按插件的部署策略设好 magpie 的设置，再 exec magpie 本体（"$@" 是 compose 里的 command）。
#
# 版本由插件按镜像摘要固定，升级走插件发布（换镜像摘要），所以：
#   noAutoUpdate —— magpie 不自动向 usemagpie.ai 查新版本、不下载；
#   noUpdatePill —— 网页顶栏不显示 Update 气泡：这个版本里点了也换不掉自己，只叫人困惑。
# 这两项是插件定死的部署策略，每次启动都重设；主人在网页里改的其它设置不受影响。
#
# 固定镜像（0.1.1327）的设置还没有命令行（`magpie settings` 是之后的版本才加的），按 magpie 写盘的
# 格式直接改 settings.json：两格缩进的对象（和 default-model.sh 读 providers.json 的假定一致）。
# 不是这个形状就不动、停下报错，不猜格式改坏主人的设置；magpie 自己对读坏的文件有上一代备份，这里不重复它。
set -eu
settings=/config/magpie/settings.json
mkdir -p "${settings%/*}"
if [ ! -s "$settings" ] || [ "$(tr -d '[:space:]' <"$settings")" = '{}' ]; then
  # 还没有设置（首次安装，或文件是空对象）：写上这两项，其余都是 magpie 的默认值。
  printf '{\n  "noAutoUpdate": true,\n  "noUpdatePill": true\n}\n' >"$settings"
else
  [ "$(head -n 1 "$settings" | tr -d '[:space:]')" = '{' ] ||
    { echo "settings.json 不是 magpie 写的缩进对象，不敢动：$settings" >&2; exit 1; }
  sed -i -E -e 's/"noAutoUpdate"[[:space:]]*:[[:space:]]*false/"noAutoUpdate": true/g' \
    -e 's/"noUpdatePill"[[:space:]]*:[[:space:]]*false/"noUpdatePill": true/g' "$settings"
  grep -q '"noAutoUpdate"' "$settings" || sed -i '1a\  "noAutoUpdate": true,' "$settings"
  grep -q '"noUpdatePill"' "$settings" || sed -i '1a\  "noUpdatePill": true,' "$settings"
fi
grep -q '"noAutoUpdate"[[:space:]]*:[[:space:]]*true' "$settings" ||
  { echo "noAutoUpdate 没设上，settings.json 里有不认识的写法：$settings" >&2; exit 1; }
grep -q '"noUpdatePill"[[:space:]]*:[[:space:]]*true' "$settings" ||
  { echo "noUpdatePill 没设上，settings.json 里有不认识的写法：$settings" >&2; exit 1; }
# 网页本体听内部端口：3430 交给 inject.pl，它在页面头插入 layout.css（固定版本没有样式设置）。
# compose 传的 --addr 换成内部端口；magpie 或代理任一退出，容器就退出，交给重启策略。
command -v perl >/dev/null || { echo "镜像里没有 perl，注入代理起不来" >&2; exit 1; }
args=
while [ $# -gt 0 ]; do
  case $1 in
    --addr)
      [ $# -ge 2 ] || { echo "--addr 没有值" >&2; exit 1; }
      shift
      args="$args --addr 127.0.0.1:3431"
      ;;
    *) args="$args $1" ;;
  esac
  shift
done
case " $args " in
  *" --addr 127.0.0.1:3431 "*) ;;
  *) echo "compose 的 command 里没有 --addr，网页本体不知道听哪" >&2; exit 1 ;;
esac
# shellcheck disable=SC2086
set -- $args
/magpie "$@" &
magpie=$!
perl /wonton/inject.pl &
proxy=$!
trap 'kill "$magpie" "$proxy" 2>/dev/null' INT TERM
# 本体的网关健康检查看不出网页入口死没死，两个进程互相看着。
( while kill -0 "$magpie" 2>/dev/null; do sleep 1; done; kill "$proxy" 2>/dev/null ) &
wait "$proxy" || true
kill "$magpie" 2>/dev/null || true
wait 2>/dev/null || true
kill -0 "$magpie" 2>/dev/null && exit 0
exit 1
