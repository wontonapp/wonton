#!/usr/bin/env bash
# 离线单测：不碰 adb/ssh，直接喂假 uiautomator dump / Matrix 响应给 smoke.d 的解析函数，
# 覆盖复用房间取名、引导页/默认面板识别、房名字面匹配和端口回退。跑法：
#   bash deploy/dev/calls/test/offline.sh
# shellcheck shell=bash
# shellcheck disable=SC2154 # STACK/log/die 等由被测文件或本文件提供。
set -uo pipefail

calls_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
log() { printf 'LOG %s\n' "$*" >&2; }
die() { printf 'die: %s\n' "$*" >&2; exit 1; }
# shellcheck source=deploy/dev/calls/smoke.d/parse.sh
source "$calls_dir/smoke.d/parse.sh"
# shellcheck source=deploy/dev/calls/smoke.d/matrix.sh
source "$calls_dir/smoke.d/matrix.sh"
# shellcheck source=deploy/dev/calls/smoke.d/ui.sh
source "$calls_dir/smoke.d/ui.sh"

STACK=test-stack
HOST=localhost
DEV_A=emulator-a
DEV_B=emulator-b
EVIDENCE=$(mktemp -d)
QFILE=$(mktemp)
trap 'rm -rf "$EVIDENCE" "$QFILE"' EXIT
room_name=""

fail=0
ok() { printf 'ok   %s\n' "$*"; }
bad() { printf 'FAIL %s\n' "$*" >&2; fail=$((fail + 1)); }
assert_eq() { # <期望> <实际> <说明>
    if [[ $1 == "$2" ]]; then ok "$3"; else bad "$3（期望 [$1] 实际 [$2]）"; fi
}
assert_contains() { # <内容> <片段> <说明>
    if [[ $1 == *"$2"* ]]; then ok "$3"; else bad "$3（[$1] 不含 [$2]）"; fi
}
assert_dies() { # <说明> <报错要带的片段> <命令…>
    local desc=$1 want=$2 out rc
    shift 2
    out=$( ( "$@" ) 2>&1 ) && rc=0 || rc=$?
    if (( rc == 0 )); then bad "$desc：没有失败"; return; fi
    assert_contains "$out" "$want" "$desc 报错提到 $want"
}

# —— 房名按字面匹配，不把 . + - 当正则 ——
XML_ROWS='<?xml version="1.0"?>
<node text="smoke.verify+1127" bounds="[24,300][1000,420]" />
<node text="smokeXverify+1127" bounds="[24,430][1000,550]" />'
room_name='smoke.verify+1127'
label=$(room_label "$XML_ROWS") || true
assert_eq 'smoke.verify+1127' "$label" 'room_label 字面命中房名'

room_name='smoke.verify+1127-不存在'
room_label "$XML_ROWS" >/dev/null 2>&1 && bad 'room_label 不该命中' || ok 'room_label 不命中返回非零'

# —— 引导页/默认机器人面板识别 ——
check_action() { # <期望> <xml 片段>
    local want=$1 xml=$2 got
    got=$(onboarding_action "$xml")
    assert_eq "$want" "$got" "onboarding_action 识别 $want"
}
check_action install '<node text="Install these apps" bounds="[0,0][1,1]" />'
check_action goto '<node text="Go to chat" bounds="[0,0][1,1]" />'
check_action next '<node text="Next" bounds="[0,0][1,1]" />'
check_action close '<node content-desc="Close panel" bounds="[0,0][1,1]" />'
check_action close '<node text="Close" bounds="[0,0][1,1]" />'
check_action '' '<node text="Hello" bounds="[0,0][1,1]" />'

# —— 复用房间：只给 ROOM_ID 时从 m.room.name 取名字；缺一个要报清楚 ——
api() { printf '%s\n' "${API_BODY:-}"; }
ROOM_ID='!room' ROOM_NAME='smoke-verify-1127'
assert_eq 'smoke-verify-1127' "$(resolve_room_reuse)" 'ROOM_ID/ROOM_NAME 都给时用 ROOM_NAME'
ROOM_NAME=''
API_BODY='{"name":"smoke-verify-1127"}'
assert_eq 'smoke-verify-1127' "$(resolve_room_reuse)" '只给 ROOM_ID 时从房间状态取名'
API_BODY='{"errcode":"M_NOT_FOUND","error":"Not found"}'
assert_dies '房间没名字' 'ROOM_NAME' resolve_room_reuse
api() { return 1; }
assert_dies 'Matrix API 失败' 'ROOM_NAME' resolve_room_reuse
api() { printf '%s\n' "${API_BODY:-}"; }
ROOM_ID=''
ROOM_NAME='smoke-verify-1127'
assert_dies '只给 ROOM_NAME' 'ROOM_ID' resolve_room_reuse
ROOM_ID='' ROOM_NAME=''
assert_dies '两个都不给' 'ROOM_ID' resolve_room_reuse

# —— 端口：名额端口优先；未占名额时回退 .dev-ports 并留日志；两处都没有要报错 ——
out=$(select_ports '18773 18774' '18888 18889' 2>/dev/null)
assert_eq 'slot 18774 18888 18889' "$out" '有占名额时用 stack-slot 端口'
errf=$(mktemp)
out=$(select_ports '18773 18774' 'call-e2e-relay 没有占名额，先 acquire' 2>"$errf")
assert_eq 'dev 18774 18773 18774' "$out" '未占名额时回退 .dev-ports'
assert_contains "$(cat "$errf")" '未占名额，使用 .dev-ports' '回退时留下日志'
rm -f "$errf"
out=$(select_ports '' 'call-e2e-relay 没有占名额，先 acquire' 2>/dev/null) && rc=0 || rc=$?
(( rc != 0 )) && ok '没有 .dev-ports 时失败' || bad '没有 .dev-ports 时仍成功'
out=$( (select_ports '' 'call-e2e-relay 没有占名额，先 acquire') 2>&1 ) || true
assert_contains "$out" '.dev-ports' '缺少 .dev-ports 报错点名'

# 假 dump：按队列逐个喂给 ui（跨命令替换，索引用文件计）。
ui() {
    local n=${#XMLQ[@]} idx
    idx=$(cat "$QFILE" 2>/dev/null || echo 0)
    (( idx >= n )) && idx=$((n - 1))
    printf '%s\n' "${XMLQ[$idx]}"
    echo $((idx + 1)) >"$QFILE"
}
ssh() { TAPS+=("$2"); }
sleep() { :; }
timeout() { shift; "$@"; }
cdp() { :; }
allow_media() { :; }
deadline_hit() { return 1; }

# —— 点击助手：房名按字面交给 grep，+ ( 等不能当正则 ——
XMLQ=( '<node text="Smoke (A)+1" bounds="[24,300][1000,420]" />' )
: >"$QFILE"; TAPS=()
tap_name emulator-a 'Smoke (A)+1' && ok 'tap_name 点得到含 + ( 的房名' || bad 'tap_name 点不到含 + ( 的房名'
assert_contains "${TAPS[*]}" 'input tap 512 360' 'tap_name 按字面命中的 bounds 点击'

: >"$QFILE"; TAPS=()
tap_text emulator-a 'Smoke (A)+1' && ok 'tap_text 点得到含 + ( 的文本' || bad 'tap_text 点不到含 + ( 的文本'
assert_contains "${TAPS[*]}" 'input tap 512 360' 'tap_text 按字面命中的 bounds 点击'

XMLQ=( '<node content-desc="Close panel" bounds="[900,100][1000,200]" />' )
: >"$QFILE"; TAPS=()
tap_desc emulator-a 'Close panel' && ok 'tap_desc 点得到 content-desc' || bad 'tap_desc 点不到 content-desc'
assert_contains "${TAPS[*]}" 'input tap 950 150' 'tap_desc 按无障碍描述的 bounds 点击'

# 集成测试用 stub，记录的是按钮文案而不是坐标。
tap_text() { TAPS+=("$2"); }
tap_name() { TAPS+=("$2"); }
tap_desc() { # 和真实现一样：没这个 desc 就失败（好让调用方继续试下一个描述）
    local xml
    xml=$(ui "$1")
    [[ $xml == *"content-desc=\"$2\""* ]] && TAPS+=("$2")
}

# —— to_list：装插件（失败后 Try again）→ Go to chat → content-desc 关面板 → 列表 ——
XMLQ=(
    '<node text="Install these apps" bounds="[300,1800][800,1900]" />'
    '<node text="Install these apps" bounds="[300,1800][800,1900]" />'
    '<node text="Install these apps" bounds="[300,1800][800,1900]" /><node text="Try again" bounds="[300,1600][800,1700]" />'
    '<node text="Go to chat" bounds="[300,1800][800,1900]" />'
    '<node content-desc="Close panel" bounds="[900,100][1000,200]" />'
    '<node content-desc="Close panel" bounds="[900,100][1000,200]" />'
    '<node content-desc="Close panel" bounds="[900,100][1000,200]" />'
    '<node content-desc="Sidebar" bounds="[0,0][100,100]" /><node text="smoke-verify-1127" bounds="[0,300][1000,420]" />'
)
room_name='smoke-verify-1127'
: >"$QFILE"
TAPS=()
to_list emulator-a && ok 'to_list 走过引导和面板后回到列表' || bad 'to_list 没回到列表'
assert_eq 'Install these apps|Try again|Go to chat|Close panel' "$(IFS='|'; printf '%s' "${TAPS[*]}")"     'to_list 依次点装插件、Try again、进聊天、关面板'

# —— to_list：引导页的 Skip for now ——
XMLQ=(
    '<node text="Skip for now" bounds="[600,1800][1000,1900]" />'
    '<node content-desc="Sidebar" bounds="[0,0][100,100]" /><node text="smoke-verify-1127" bounds="[0,300][1000,420]" />'
)
: >"$QFILE"
TAPS=()
to_list emulator-a && ok 'to_list 点 Skip for now 跳过引导' || bad 'to_list 没跳过引导'
assert_eq 'Skip for now' "$(IFS='|'; printf '%s' "${TAPS[*]}")" 'to_list 命中 Skip for now'

# —— open_call：列表里按当前房名（不一定是 call-smoke-*）进房找入口 ——
XMLQ=(
    '<node text="smoke-verify-1127" bounds="[24,300][1000,420]" />'
    '<node text="Element Call | smoke-verify-1127" bounds="[0,0][1080,200]" />'
)
: >"$QFILE"
TAPS=()
open_call emulator-a && ok 'open_call 用当前房名回退进房' || bad 'open_call 没进房'
assert_contains "${TAPS[*]}" 'smoke-verify-1127' 'open_call 点到了当前房名'

if (( fail )); then
    printf '\n%d 项失败\n' "$fail" >&2
    exit 1
fi
printf '\n全部通过\n'
