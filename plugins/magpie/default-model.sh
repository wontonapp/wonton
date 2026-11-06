#!/bin/sh
# 维护默认路由组 wonton-default（注入给各插件的默认模型），规则见 ADR-0008 的“补充：默认模型”：
# 用 magpie 自己的“Cheap first”模板挑出的模型（便宜的在前，最强的在后，价格来自 models.dev 的模型目录）；挑不出来时
# （还没有 provider、只有一个模型、几个模型一样贵）用全部模型（*），按 provider 添加的先后和各自列出的顺序。
# provider、模型列表或模型目录变了就重新挑。主人在“模型设置”里改过这个组就不再动它；删掉它，就按规则重建。
#
# 主人改没改过，看 providers.json 里这个组的那一段和上次写下的是否一样：加删 provider 时 magpie 不改组的定义
# （magpie 0.1.1327 实测），主人在网页或命令行里改了什么都会改它。
group=wonton-default
providers=/config/magpie/providers.json
catalog=/config/cache/magpie/models.json
state=/config/wonton
# 健康检查看它：上一次维护成功了才有。
ok=/tmp/default-model.ok
rm -f "$ok"
mkdir -p "$state" || exit 1
# sh 是容器里的 1 号进程，不设 trap 就收不到 docker stop 的 TERM，要等 10 秒被强杀。
trap 'exit 0' TERM INT

# providers.json 里这个组的那一段（magpie 按两格缩进写，组在 "groups" 下缩进四格）；没有这个组时为空。后面还有别的组时
# 结尾是 `},`，去掉这个逗号：主人在它后面加删组不算改了它。
block() {
  [ -f "$providers" ] || return 0
  awk -v id="\"id\": \"$group\"," '
    /^  "groups": \[/ { inside = 1; next }
    inside && /^  [\]}]/ { exit }
    inside && /^    \{/ { buf = $0; next }
    inside { buf = buf "\n" $0 }
    inside && /^    \}/ { if (index(buf, id)) { sub(/,$/, "", buf); print buf; exit } }
  ' "$providers"
}

# 要写进组里的模型，逗号分隔；“Cheap first”挑不出来时是 *。
# 模型目录由 magpie 的服务进程下载，CLI 不下载。没有目录时模板照样挑，但不看价格（magpie 0.1.1327 实测：OpenAI 预设下
# 挑出 gpt-3.5-turbo → gpt-4-0613），所以当作挑不出来。
wanted() {
  # 自定义接入显式指定的模型优先；reconcile 仍先检查主人是否改过组。
  if [ -s "$state/custom-model" ]; then
    custom=$(cat "$state/custom-model") || return 1
    custom_id=${custom%%/*}
    if magpie provider "$custom_id" >/dev/null 2>&1; then
      # 自定义 provider 只暴露所填的一个模型；按 provider 匹配，避免模型名的逗号和 :effort 被组语法解释。
      printf '%s/*\n' "$custom_id"
      return 0
    fi
  fi
  [ -s "$catalog" ] || { echo '*'; return 0; }
  templates=$(magpie group template) || return 1
  pair=$(printf '%s\n' "$templates" | sed -n '/^thrifty /{n;p;q;}')
  case $pair in
    "" | *"can't be made"*) echo '*' ;;
    *) printf '%s\n' "$pair" | sed 's/^ *//; s/ → /,/g' ;;
  esac
}

# 把组写成 $1。组在时按 id 改；不在时先用一个没人用的名字建出来，再改成这个 id 和名字：magpie 的 group add 按名字找组、
# 名字在用就替换那个组，group set 找不到这个 id 时也按名字找，主人改了 id、还叫这个名字的组会被改掉（magpie 0.1.1327 实测）。
write() {
  if [ -n "$current" ]; then
    magpie group set "$group" "models=$1" routing=order >/dev/null
  else
    new="$group-$(date +%s)"
    magpie group add "$new" "models=$1" routing=order >/dev/null || return 1
    magpie group set "$new" id="$group" name="$group" >/dev/null && return 0
    # 改不成就删掉，免得每次重试都多出一个组。
    magpie group rm "$new" >/dev/null
    return 1
  fi
}

reconcile() {
  current=$(block) || return 1
  if [ -n "$current" ] && [ "$current" != "$(cat "$state/applied" 2>/dev/null)" ]; then
    if [ -z "${left:-}" ]; then
      echo "主人改过默认路由组 $group，不再自动调整；删掉它就按规则重建"
      left=1
    fi
    return 0
  fi
  left=
  models=$(wanted) || return 1
  if [ -n "$current" ] && [ "$models" = "$(cat "$state/wanted" 2>/dev/null)" ]; then
    return 0
  fi
  # 挑模型要读几 MB 的模型目录，主人可能在这期间改了组：改了就不写，下一轮当作主人改过。
  [ "$(block)" = "$current" ] || return 0
  write "$models" || return 1
  block >"$state/applied" && echo "$models" >"$state/wanted" || return 1
  echo "默认路由组 $group：$models"
}

# provider、模型列表和模型目录的缓存变了才重新算：magpie 每次都要读几 MB 的模型目录。
signature() {
  stat -c '%n %Y %s' "$providers" "$catalog" "$state/custom-model" /config/cache/magpie/models \
    /config/cache/magpie/models/* 2>/dev/null | cksum
}

last=
while :; do
  now=$(signature)
  if [ "$now" != "$last" ]; then
    if reconcile; then
      last=$now
      : >"$ok"
    else
      rm -f "$ok"
      echo "维护默认路由组 $group 失败，两秒后重试" >&2
    fi
  fi
  # 放到后台再 wait：TERM 到了 wait 立刻返回，trap 马上退出。
  sleep 2 &
  wait $!
done
