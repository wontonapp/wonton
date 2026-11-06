#!/bin/sh
# 列出、保存和隔离测试模型服务商；自定义输入为 {base_url,model,key} 单行 JSON。
# 秘密只从标准输入进来、只交给 magpie，
# 命令输出都留在 shell 变量里，不进 stdout 和 stderr。
# 实测行为基于 magpie 0.1.1327（ghcr.io/yetone/magpie@sha256:85e5b25a…）：
# - presets 和 providers 是给人看的表格，没有 JSON 输出：行首有缩进，标题可含空格。
#   预设行以 "magpie provider add <id>" 结尾；provider 行在 key 掩码（●）前是域名，再往前是 id。
# - provider add 同名不更新，会新建第二个（id 带 -2 后缀）；add 永远退出 0，key 被拒或
#   连不上只反映在输出的警告里，不能当保存成功，必须再跑 provider test。
# - provider test 发真实请求：被拒时输出 "✗ chat  401 · …"，退出 1；断网时 add 拉不到模型
#   列表，test 输出 "✗ chat  no model to try: …"，或连接错误，也退出 1。
# - provider set <id> id=<新id> 改名；目标名已存在时退出 1，所以换 key 要先删旧的再改名。
# - 自定义 add 的 url/key/models 字段可用；断连时 add 仍为 0，models 保留显式模型；
#   provider test <id> <model> 发该模型请求并报 connection refused。
# - add 拒绝已经存在的同 URL+key，即使 id 不同；不改旧配置、不伪装成保存成功。
#   models= 把逗号拆成列表，provider models <id> <model> 则把一个 argv 当一个模型。
# - 独立 HOME/XDG 下的命令只改隔离目录；200、401、断连、20 秒超时和配置错误后，
#   在线 providers.json 与 providers 输出不变。真实网关凭据请求默认组可到达自定义模型。
# 自定义保存使用唯一 id，成功/断连保留，拒绝/内部错误删除；custom-model 提示默认组优先
# 使用精确模型，但 default-model.sh 仍尊重主人手改的组。test 两种模式不碰在线配置。
set -eu

PROVIDER_TIMEOUT=20

usage() {
  echo "用法:$0 list | save <预设ID> | test <ID> | custom-save | custom-test" >&2
  exit 2
}

# 预设 JSON。每行含 "magpie provider add <id>"，标题在它前面，id 是它后面的第一个词。
presets_json() {
  table=$(magpie presets 2>/dev/null) || {
    echo "magpie presets 执行失败" >&2
    return 1
  }
  json=$(printf '%s\n' "$table" | awk '
    {
      idx = index($0, "magpie provider add ")
      if (idx == 0) next
      id = substr($0, idx + 20)
      sub(/[[:space:]].*/, "", id)
      left = substr($0, 1, idx - 1)
      sub(/[[:space:]]+$/, "", left)
      # left 是 "<标题可含空格>  <id>"：最后一列是 id，往前跳过多空格就是标题。
      m = match(left, /[[:space:]][[:space:]]+[^[:space:]]+$/)
      title = m ? substr(left, 1, m - 1) : left
      sub(/^[[:space:]]+/, "", title)
      sub(/[[:space:]]+$/, "", title)
      gsub(/\\/, "\\\\", title); gsub(/"/, "\\\"", title)
      if (n++) printf ","
      printf "{\"id\":\"%s\",\"title\":\"%s\"}", id, title
    }
  ')
  # 解析不到任何预设说明表格格式变了，如实报错，不给 App 一个"没有服务商"的假象。
  [ -n "$json" ] || {
    echo "magpie presets 的输出里没解析到任何预设" >&2
    return 1
  }
  printf '%s\n' "$json"
}

preset_exists() {
  table=$(magpie presets 2>/dev/null) || {
    echo "magpie presets 执行失败" >&2
    return 1
  }
  printf '%s\n' "$table" | awk -v wanted="$1" '
    {
      idx = index($0, "magpie provider add ")
      if (idx == 0) next
      id = substr($0, idx + 20)
      sub(/[[:space:]].*/, "", id)
      if (id == wanted) found = 1
    }
    END { exit !found }
  '
}

# providers 表里的所有 id。行的形状是 "<标题可含空格>  <id>  <域名或地址>  ● <key 掩码> …"。
# 域名列不检查含点：localhost:11434 这类自定义地址也合法（实测标题里的词不会顶到 id 位）。
provider_ids() {
  table=$(magpie providers 2>/dev/null) || {
    echo "magpie providers 执行失败" >&2
    return 1
  }
  printf '%s\n' "$table" | awk '
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      idx = index(line, "●")
      if (idx == 0) next
      left = substr(line, 1, idx - 1)
      sub(/[[:space:]]+$/, "", left)
      n = split(left, w, /[[:space:]]+/)
      if (n >= 2 && w[n - 1] ~ /^[a-z0-9][a-z0-9.-]*$/) print w[n - 1]
    }
  '
}

# providers 表里一个 id 的标题列：● 前去掉末尾的 host 和 id 两列。
provider_title() {
  table=$(magpie providers 2>/dev/null) || return 1
  printf '%s\n' "$table" | awk -v wanted="$1" '
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      idx = index(line, "●")
      if (idx == 0) next
      left = substr(line, 1, idx - 1)
      sub(/[[:space:]]+$/, "", left)
      n = split(left, w, /[[:space:]]+/)
      if (n >= 3 && w[n - 1] == wanted) {
        title = left
        sub(/[[:space:]]+[^[:space:]]+[[:space:]]+[^[:space:]]+$/, "", title)
        sub(/^[[:space:]]+/, "", title)
        gsub(/\\/, "\\\\", title); gsub(/"/, "\\\"", title)
        print title
        exit
      }
    }
  '
}

provider_id() {
  ids=$(provider_ids) || return 1
  for known in $ids; do
    if [ "$known" = "$1" ]; then
      printf '%s\n' "$1"
      return 0
    fi
  done
  # 没有这个 id 也是正常状态：输出空。
  return 0
}

list() {
  presets=$(presets_json) || exit 1
  configured=""
  ids=$(provider_ids) || exit 1
  for id in $ids; do
    # configured 就是 providers 表里的 id（实测 0.1.1327：已添加的 vendor 从 presets 表
    # 消失，无法回表校验；非预设的自定义 id 也算已配置）。
    case ",$configured," in
      *,"$id",*) ;;
      *) configured="${configured:+$configured,}\"$id\"" ;;
    esac
    # 核心的约定是 configured 的每一项都在 presets 里，把缺的补上（标题取 providers
    # 表的标题列，取不到就用 id），这样已配置的 provider 和主人自建的自定义 provider
    # 都会出现在 App 的列表里。
    case "$presets" in
      *"\"id\":\"$id\""*) ;;
      *)
        # 实测 0.1.1327：非预设行的名字列后面跟着 magpie 自己加的 " custom" 标记。
        title=$(provider_title "$id")
        title=${title% custom}
        [ -n "$title" ] || title="$id"
        presets="$presets,{\"id\":\"$id\",\"title\":\"$title\"}"
        ;;
    esac
  done
  printf '{"presets":[%s],"configured":[%s]}\n' "$presets" "$configured"
}

# 诊断只留在变量里。被拒返回 3，连不上返回 4，其他一律 1；timeout 的 124 也按连不上算。
# 实测（0.1.1327）：被拒是 "✗ chat  401 · …"，401/403 带空格边界，不会撞耗时和地址里的
# 数字；断网时 add 拉不到模型列表，test 输出 "✗ chat  no model to try: …"，或
# "network is unreachable" 一类连接错误。
test_provider() {
  output=$(timeout "${PROVIDER_TIMEOUT}s" magpie provider test "$@" 2>&1) && return 0
  status=$?
  [ "$status" -eq 124 ] && return 4
  case "$output" in
    *" 401 "*|*" 403 "*|*Unauthorized*|*unauthorized*|*Forbidden*|*forbidden*|*"api key"*|*"API key"*) return 3 ;;
    *"no model to try"*|*unreachable*|*"connection refused"*|*"connection reset"*|*"timed out"*|*timeout*|*"Could not resolve"*|*"no such host"*) return 4 ;;
    *) return 1 ;;
  esac
}

save() {
  id=$1
  old_id=$(provider_id "$id") || exit 1
  # 已添加的 vendor 会从 presets 表里消失（实测 0.1.1327），所以已是 provider 的 id 也算合法。
  preset_exists "$id" || [ -n "$old_id" ] || exit 2
  # 自定义 provider（主人自建，不在 presets）走不到 add：magpie 把 `add <非预设> <key>`
  # 当 `add <name> k=v` 拒掉（"expected key=value"），退出 1，原来的 provider 和 key
  # 原样不动，核心把它报成网关出错。
  IFS= read -r key || true
  [ -n "$key" ] || exit 2

  if [ -n "$old_id" ]; then
    ids=$(provider_ids) || exit 1
    # before/after 是换行分隔，转成空格分隔，下面的 case 才能按 " <id> " 匹配。
    before=$(printf '%s\n' "$ids" | tr '\n' ' ')
    # 同名 add 会新建 deepseek-2 且永远退出 0，成败只能看 test：从前后差集找新建的 id。
    magpie provider add "$id" "$key" >/dev/null 2>&1 || exit 1
    ids=$(provider_ids) || exit 1
    after=$(printf '%s\n' "$ids" | tr '\n' ' ')
    candidate=""
    for candidate_id in $after; do
      case " $before " in
        *" $candidate_id "*) ;;
        *) candidate=$candidate_id; break ;;
      esac
    done
    [ -n "$candidate" ] || exit 1

    # 不能写成 `if test_provider …; then …; fi` 后再取 $?：条件假且无 else 时
    # if 语句的退出码是 0，不是 test 的。
    status=0
    test_provider "$candidate" || status=$?
    # set 改名不改显示名（实测 0.1.1327，改完表格里还是 "DeepSeek 2"），换 key
    # 前记下旧 provider 的名字，改名后恢复。
    old_name=$(provider_title "$old_id")
    case "$status" in
      0)
        magpie provider rm "$old_id" >/dev/null 2>&1 || exit 1
        # 先删旧的再改名：旧 provider 的 id 就是预设 id，直接改会撞名（set 报 already）。
        magpie provider set "$candidate" "id=$id" >/dev/null 2>&1 || exit 1
        if [ -n "$old_name" ]; then
          magpie provider set "$id" "name=$old_name" >/dev/null 2>&1 || exit 1
        fi
        ;;
      4)
        # 连不上：新 key 留下（同样先删旧的再改名），核心如实报"已保存、结果不确定"。
        magpie provider rm "$old_id" >/dev/null 2>&1 || exit 1
        magpie provider set "$candidate" "id=$id" >/dev/null 2>&1 || exit 1
        if [ -n "$old_name" ]; then
          magpie provider set "$id" "name=$old_name" >/dev/null 2>&1 || exit 1
        fi
        ;;
      3|*)
        # key 被拒：删掉新建的，旧 key 原样不动。
        magpie provider rm "$candidate" >/dev/null 2>&1 || :
        ;;
    esac
    return "$status"
  fi

  magpie provider add "$id" "$key" >/dev/null 2>&1 || exit 1
  new_id=$(provider_id "$id") || exit 1
  [ -n "$new_id" ] || exit 1
  # 不能写成 `if test_provider …; then return 0; fi` 后再取 $?：条件假且无 else
  # 时 if 语句的退出码是 0，不是 test 的。
  status=0
  test_provider "$new_id" || status=$?
  case "$status" in
    0) ;; # 测试通过：key 保存成功
    4) ;; # 连不上：key 留着，核心如实报不确定
    *) magpie provider rm "$new_id" >/dev/null 2>&1 || : ;; # 没验证通过的 key 不留
  esac
  return "$status"
}

# 测试在独立配置树内运行，后台默认组维护器看不到候选项；所有退出路径删除整个树。
# XDG 只在子 shell 隔离，custom_save 必须使用父进程原值。
# shellcheck disable=SC2030
isolated() (
  umask 077
  scratch=$(mktemp -d /tmp/wonton-provider.XXXXXXXXXX) || exit 1
  trap 'rm -rf "$scratch"' EXIT
  trap 'exit 1' HUP INT TERM
  original=${XDG_CONFIG_HOME:-/config}/magpie/providers.json
  export HOME="$scratch/home" XDG_CONFIG_HOME="$scratch/config"
  export XDG_CACHE_HOME="$scratch/cache" XDG_DATA_HOME="$scratch/data"
  export XDG_STATE_HOME="$scratch/state" MAGPIE_NO_ACCOUNTS=1
  mkdir -p "$XDG_CONFIG_HOME/magpie" "$HOME"
  probe="wonton-probe-$(basename "$scratch" | tr '[:upper:]' '[:lower:]' | tr '.' '-')"
  if [ "$1" = test ]; then
    [ ! -f "$original" ] || cp "$original" "$XDG_CONFIG_HOME/magpie/providers.json"
    known=$(provider_id "$2") || exit 1
    if [ -n "$known" ]; then
      magpie provider set "$2" "id=$probe" "key=$key" >/dev/null 2>&1 || exit 1
    else
      preset_exists "$2" || exit 2
      status=0
      timeout "${PROVIDER_TIMEOUT}s" magpie provider add "$2" "$key" "id=$probe" >/dev/null 2>&1 || status=$?
      case "$status" in 0) ;; 124) exit 4 ;; *) exit 1 ;; esac
    fi
  else
    create_custom "$probe" || exit $?
  fi
  if [ "$1" = test ]; then test_provider "$probe"; else test_provider "$probe" "$model"; fi
)

# 镜像没有 jq：严格解码三个字符串字段，不 eval、不按引号切 JSON。
custom_input() {
  parsed=$(LC_ALL=C awk '
    function fail() { bad=1; exit 2 }
    function ws() { while(p<=length(s) && substr(s,p,1) ~ /[ \t\r\n]/) p++ }
    function hex(  i,c,n) {
      n=0
      for(i=0;i<4;i++) {
        if(p>length(s)) fail()
        c=index("0123456789abcdef",tolower(substr(s,p++,1)))-1
        if(c<0) fail()
        n=n*16+c
      }
      return n
    }
    function utf(n) {
      if(n==0 || n==10 || n==13) fail()
      if(n<128) return sprintf("%c",n)
      if(n<2048) return sprintf("%c%c",192+int(n/64),128+n%64)
      if(n<65536) return sprintf("%c%c%c",224+int(n/4096),128+int(n/64)%64,128+n%64)
      return sprintf("%c%c%c%c",240+int(n/262144),128+int(n/4096)%64,128+int(n/64)%64,128+n%64)
    }
    function str(  c,e,n,lo,v) {
      if(substr(s,p++,1)!="\"") fail()
      v=""
      while(p<=length(s)) {
        c=substr(s,p++,1)
        if(c=="\"") return v
        if(c ~ /[[:cntrl:]]/) fail()
        if(c=="\\") {
          e=substr(s,p++,1)
          if(e=="\"" || e=="\\" || e=="/") c=e
          else if(e=="t") c=sprintf("%c",9)
          else if(e=="b") c=sprintf("%c",8)
          else if(e=="f") c=sprintf("%c",12)
          else if(e=="u") {
            n=hex()
            if(n>=55296 && n<=56319) {
              if(substr(s,p,2)!="\\u") fail()
              p+=2; lo=hex()
              if(lo<56320 || lo>57343) fail()
              n=65536+(n-55296)*1024+lo-56320
            } else if(n>=56320 && n<=57343) fail()
            c=utf(n)
          } else fail()
        }
        v=v c
      }
      fail()
    }
    { if(NR>1) fail(); s=$0 }
    END {
      if(bad) exit 2
      p=1; ws(); if(substr(s,p++,1)!="{") fail()
      for(i=1;i<=3;i++) {
        ws(); k=str(); ws(); if(substr(s,p++,1)!=":") fail()
        if(k!="base_url" && k!="model" && k!="key") fail()
        if(k in a) fail()
        ws(); a[k]=str(); ws()
        if(substr(s,p++,1)!=(i==3 ? "}" : ",")) fail()
      }
      ws(); if(p<=length(s)) fail()
      if(a["base_url"]!~/^https?:\/\/[^\/[:space:]]+/ || a["base_url"] ~ /[[:space:][:cntrl:]]/) fail()
      if(a["model"]=="" || a["model"] ~ /[[:space:][:cntrl:]]/ || a["key"]=="") fail()
      print a["base_url"]; print a["model"]; print a["key"]
    }
  ') || return 2
  base_url=$(printf '%s\n' "$parsed" | sed -n '1p')
  model=$(printf '%s\n' "$parsed" | sed -n '2p')
  key=$(printf '%s\n' "$parsed" | sed -n '3p')
}

create_custom() {
  status=0
  # 不给 name 时 magpie 用 id 当名字，App 里会显示成 wonton-custom-…，所以写成“模型 · 主机”。
  host=${base_url#*://}
  host=${host%%/*}
  timeout "${PROVIDER_TIMEOUT}s" magpie provider add "$1" "id=$1" "name=$model · $host" \
    "url=$base_url" "models=$model" "key=$key" >/dev/null 2>&1 || status=$?
  case "$status" in 0) ;; 124) return 4 ;; *) return 1 ;; esac
  # models= 用逗号分隔；models 子命令把每个 argv 当一个完整模型名。
  case "$model" in
    *,*) magpie provider models "$1" "$model" >/dev/null 2>&1 || return 1 ;;
  esac
}

# shellcheck disable=SC2031
custom_save() (
  umask 077
  state=${XDG_CONFIG_HOME:-/config}/wonton
  mkdir -p "$state" || exit 1
  reservation=$(mktemp -d "$state/custom.XXXXXXXXXX") || exit 1
  candidate="wonton-custom-$(basename "$reservation" | tr '[:upper:]' '[:lower:]' | tr '.' '-')"
  keep=0
  # EXIT trap 间接调用。
  # shellcheck disable=SC2317
  cleanup_custom() {
    result=$?
    if [ "$keep" = 0 ]; then
      magpie provider rm "$candidate" >/dev/null 2>&1 || result=1
    fi
    rm -rf "$reservation" || result=1
    exit "$result"
  }
  trap cleanup_custom EXIT
  trap 'exit 1' HUP INT TERM
  existing=$(provider_id "$candidate") || { keep=1; exit 1; }
  [ -z "$existing" ] || { keep=1; exit 1; }
  status=0
  create_custom "$candidate" || status=$?
  [ "$status" -ne 0 ] || test_provider "$candidate" "$model" || status=$?
  case "$status" in
    0|4)
      # 连不上仍保存，和 save 合同一致；精确模型提示不含 key。
      [ -n "$(provider_id "$candidate")" ] || exit 1
      printf '%s/%s\n' "$candidate" "$model" >"$reservation/model"
      mv "$reservation/model" "$state/custom-model" || exit 1
      keep=1
      ;;
  esac
  exit "$status"
)

case "${1:-}" in
list)
  [ "$#" -eq 1 ] || usage
  list
  ;;
save)
  [ "$#" -eq 2 ] || usage
  save "$2"
  ;;
test)
  [ "$#" -eq 2 ] || usage
  key=
  IFS= read -r key || true
  [ -n "$key" ] || exit 2
  isolated test "$2"
  ;;
custom-test|custom-save)
  [ "$#" -eq 1 ] || usage
  custom_input || exit 2
  if [ "$1" = custom-test ]; then isolated custom; else custom_save; fi
  ;;
*) usage ;;
esac

