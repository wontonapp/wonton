#!/usr/bin/env bash
# MatrixRTC/Element Call Android smoke test (ADR-0096).
#
# The script owns only the stack named by CALL_PROJECT and the two explicitly
# supplied Android devices. It installs the local call plugin, opens its widget
# through a Wonton app link, records CDP WebRTC stats, and checks inbound RTP.
# Account setup is deliberately a prerequisite: two devices must be paired as
# different accounts and logged into the same encrypted room.
#
# Required:
#   CALL_PLUGIN=/path/to/wonton-plugin-call
#   CALL_SERIAL_A=<adb serial> CALL_SERIAL_B=<adb serial>
#   CALL_ROOM='!room:server'
#   CALL_APK=/path/to/androidApp-debug.apk (or CALL_BUILD=1)
#
# One path at a time:
#   CALL_MODE=udp   deploy/dev/core/call-e2e.sh
#   CALL_MODE=tcp   deploy/dev/core/call-e2e.sh
#   CALL_MODE=relay deploy/dev/core/call-e2e.sh
# Relay mode does not force or verify the iroh path; check both devices' actual
# tunnel paths independently. Also verify the selected ICE transport and leave.
#
# The default artifact directory is outside the repository:
#   ~/wonton-dev/research/call-e2e/<mode>-<timestamp>/
main() {
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"
cdp="$here/call-cdp.py"

mode="${CALL_MODE:-udp}"
project="${CALL_PROJECT:-wonton-call-e2e}"
instance="${CALL_INSTANCE:-call-e2e}"
port="${CALL_PORT:-18680}"
tunnel_port="${CALL_TUNNEL_PORT:-18682}"
slot="${CALL_SLOT_NAME:-call-e2e}"
plugin="${CALL_PLUGIN:-}"
serial_a="${CALL_SERIAL_A:-}"
serial_b="${CALL_SERIAL_B:-}"
room="${CALL_ROOM:-}"
apk="${CALL_APK:-$repo/apps/compose/androidApp/build/outputs/apk/debug/androidApp-debug.apk}"
duration="${CALL_DURATION:-30}"
artifact_root="${CALL_ARTIFACT_DIR:-$HOME/wonton-dev/research/call-e2e}"
existing_stack="${CALL_EXISTING_STACK:-0}"
reuse_plugin="${CALL_REUSE_PLUGIN:-0}"
build="${CALL_BUILD:-0}"
keep_stack="${CALL_KEEP_STACK:-0}"

case "$mode" in
  udp|tcp|relay) ;;
  *) echo "call-e2e: CALL_MODE must be udp, tcp, or relay" >&2; exit 2 ;;
esac
[[ -n "$plugin" && -e "$plugin" ]] || { echo "call-e2e: CALL_PLUGIN is missing" >&2; exit 2; }
[[ -n "$serial_a" && -n "$serial_b" ]] || { echo "call-e2e: CALL_SERIAL_A and CALL_SERIAL_B are required" >&2; exit 2; }
[[ "$serial_a" != "$serial_b" ]] || { echo "call-e2e: two different Android devices are required" >&2; exit 2; }
[[ -n "$room" ]] || { echo "call-e2e: CALL_ROOM is required" >&2; exit 2; }
command -v adb >/dev/null || { echo "call-e2e: adb is required" >&2; exit 2; }
command -v jq >/dev/null || { echo "call-e2e: jq is required" >&2; exit 2; }
[[ -f "$cdp" ]] || { echo "call-e2e: missing $cdp" >&2; exit 2; }
[[ "$reuse_plugin" != 1 || "$existing_stack" == 1 ]] || {
  echo "call-e2e: CALL_REUSE_PLUGIN=1 requires CALL_EXISTING_STACK=1" >&2; exit 2;
}

run_id="${mode}-$(date +%Y%m%d-%H%M%S)-$$"
export WONTON_DEV_DATA="${CALL_DATA_DIR:-$here/.data/call-e2e-$run_id}"
artifacts="$artifact_root/$run_id"
mkdir -p "$artifacts"
log="$artifacts/run.log"
exec > >(tee -a "$log") 2>&1

own_stack=0
plugin_copy=""
forward_a=""
forward_b=""
record_pids=()
record_serials=()
record_dirs=()
record_names=()

cleanup() {
  local status=$?
  set +e
  if ! finish_recordings; then status=1; fi
  [[ -z "$forward_a" ]] || adb -s "$serial_a" forward --remove "tcp:$forward_a" >/dev/null 2>&1
  [[ -z "$forward_b" ]] || adb -s "$serial_b" forward --remove "tcp:$forward_b" >/dev/null 2>&1
  if [[ -n "$plugin_copy" ]]; then rm -rf -- "$plugin_copy"; fi
  if ((own_stack)); then
    if [[ "$keep_stack" != 1 ]]; then
      if WONTON_DEV_PROJECT="$project" WONTON_DEV_INSTANCE="$instance" \
        WONTON_DEV_PORT="$port" WONTON_DEV_TUNNEL_PORT="$tunnel_port" \
        WONTON_DEV_DOMAIN="$WONTON_DEV_DOMAIN" WONTON_DEV_DATA="$WONTON_DEV_DATA" \
        "$dev" reset; then
        "$HOME/wonton-dev/stack-slot.sh" release "$slot" || status=1
      else
        echo "call-e2e: stack cleanup failed; slot $slot remains reserved" >&2
        status=1
      fi
    else
      echo "call-e2e: stack and slot $slot kept for inspection"
    fi
  fi
  if ((status == 0)); then
    echo "call-e2e: artifacts $artifacts"
  else
    echo "call-e2e: failed; artifacts $artifacts" >&2
  fi
  exit "$status"
}

export WONTON_DEV_PROJECT="$project"
export WONTON_DEV_INSTANCE="$instance"
export WONTON_DEV_PORT="$port"
export WONTON_DEV_TUNNEL_PORT="$tunnel_port"
export WONTON_DEV_DOMAIN="${CALL_DOMAIN:-${instance}.wonton.localhost}"
export WONTON_DEV_DEFAULT_PLUGINS="${WONTON_DEV_DEFAULT_PLUGINS-}"
export WONTON_DEV_TUNNEL_BIND="${WONTON_DEV_TUNNEL_BIND:-0.0.0.0}"

adb_ready() {
  local serial="$1"
  adb -s "$serial" wait-for-device >/dev/null
  adb -s "$serial" shell getprop sys.boot_completed | tr -d '\r' | grep -qx 1
}

plugin_compose() {
  if [[ -d "$plugin" ]]; then
    printf '%s/docker-compose.yml\n' "${plugin%/}"
  else
    printf '%s\n' "$plugin"
  fi
}

prepare_tcp_plugin() {
  local source
  source="$(plugin_compose)"
  [[ -f "$source" ]] || { echo "call-e2e: no docker-compose.yml at $source" >&2; exit 1; }
  plugin_copy="$(mktemp -d "${TMPDIR:-/tmp}/wonton-call-tcp.XXXXXX")"
  cp -a "$(dirname "$source")/." "$plugin_copy/"
  python3 - "$plugin_copy/docker-compose.yml" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
line = "      - { service: livekit, port: 7882, protocol: udp }\n"
if line not in text:
    raise SystemExit("call-e2e: TCP mode could not find the UDP tunnel declaration")
path.write_text(text.replace(line, ""))
PY
}

install_plugin() {
  local package result state rc stderr
  package="$plugin"
  if [[ "$mode" == tcp ]]; then
    prepare_tcp_plugin
    package="$plugin_copy"
  fi
  echo "== install call plugin ($mode)"
  WONTON_DEV_PROJECT="$project" WONTON_DEV_INSTANCE="$instance" \
    WONTON_DEV_PORT="$port" WONTON_DEV_TUNNEL_PORT="$tunnel_port" \
    WONTON_DEV_DOMAIN="$WONTON_DEV_DOMAIN" "$dev" uninstall app.element-call --purge >/dev/null 2>&1 || true
  stderr="$artifacts/install.stderr"
  set +e
  result="$(WONTON_DEV_PROJECT="$project" WONTON_DEV_INSTANCE="$instance" \
    WONTON_DEV_PORT="$port" WONTON_DEV_TUNNEL_PORT="$tunnel_port" \
    WONTON_DEV_DOMAIN="$WONTON_DEV_DOMAIN" "$dev" install "$package" --yes 2>"$stderr")"
  rc=$?
  set -e
  printf '%s\n' "$result" >"$artifacts/install.json"
  if ((rc != 0)); then
    echo "call-e2e: plugin install failed (exit $rc); see $artifacts/install.json and $stderr" >&2
    cat "$stderr" >&2
    exit "$rc"
  fi
  state="$(jq -r '.status.state // empty' "$artifacts/install.json")"
  [[ "$state" == running ]] || {
    echo "call-e2e: plugin install did not become running: $result" >&2
    exit 1
  }
}

check_installed_plugin() {
  local response="$artifacts/installed.json"
  "$dev" api GET /_wonton/api/plugins >"$response"
  jq -e --arg mode "$mode" '
    .plugins[] | select(.id == "app.element-call" and .status.state == "running"
      and .matrix_widget != null
      and (if $mode == "tcp" then
        any(.tunnel_ports[]?; .port == 7881 and .protocol == "tcp")
        and (any(.tunnel_ports[]?; .port == 7882 and .protocol == "udp") | not)
      else any(.tunnel_ports[]?; .port == 7882 and .protocol == "udp") end))
  ' "$response" >/dev/null || {
    echo "call-e2e: installed plugin does not expose a running widget with the requested ports" >&2
    return 1
  }
}

build_apk() {
  [[ "$build" == 1 ]] || return 0
  echo "== build Android debug APK"
  (cd "$repo" && ANDROID_TASKS=":androidApp:assembleDebug" nice -n 19 "$HOME/wonton-dev/android-build.sh")
}

install_apk() {
  [[ -f "$apk" ]] || { echo "call-e2e: APK not found: $apk (set CALL_APK or CALL_BUILD=1)" >&2; exit 1; }
  echo "== install APK"
  for serial in "$serial_a" "$serial_b"; do
    adb_ready "$serial"
    adb -s "$serial" install -r "$apk" >/dev/null
    adb -s "$serial" shell pm grant app.wonton.android android.permission.CAMERA >/dev/null 2>&1 || true
    adb -s "$serial" shell pm grant app.wonton.android android.permission.RECORD_AUDIO >/dev/null 2>&1 || true
  done
}


launch_widget() {
  local serial="$1"
  local encoded
  encoded="$(python3 - "$room" <<'PY'
from urllib.parse import quote
import sys
print(quote(sys.argv[1], safe=""))
PY
)"
  adb -s "$serial" shell am start -a android.intent.action.VIEW \
    -d "wonton://tab/widget/app.element-call/$encoded" >/dev/null
}

forward_webview() {
  local serial="$1" pid socket_name
  pid="$(adb -s "$serial" shell pidof app.wonton.android | tr -d '\r')"
  [[ -n "$pid" ]] || { echo "call-e2e: app is not running on $serial" >&2; exit 1; }
  socket_name="webview_devtools_remote_${pid%% *}"
  adb -s "$serial" forward tcp:0 "localabstract:$socket_name"
}

check_stats() {
  local file="$1"
  jq -e 'select(any(.inbound[]?; .kind == "audio" and (.packetsReceived // 0) > 0))' "$file" >/dev/null || {
    echo "call-e2e: no inbound audio RTP packets in $file" >&2
    return 1
  }
  jq -e 'select(any(.inbound[]?; .kind == "video" and (.packetsReceived // 0) > 0))' "$file" >/dev/null || {
    echo "call-e2e: no inbound video RTP packets in $file" >&2
    return 1
  }
}


record_screen() {
  local serial="$1" name="$2"
  local remote="/sdcard/wonton-call-$run_id-$name"
  adb -s "$serial" shell "
    mkdir -p '$remote' || exit 1
    n=0
    while [ ! -f '$remote/stop' ]; do
      n=\$((n + 1))
      file=\$(printf '%04d.mp4' \"\$n\")
      screenrecord --size 720x1280 --time-limit 180 '$remote/'\"\$file\" &
      child=\$!
      echo \"\$child\" > '$remote/pid'
      [ -f '$remote/stop' ] && kill -INT \"\$child\" 2>/dev/null || true
      wait \"\$child\"
      rc=\$?
      rm -f '$remote/pid'
      [ -f '$remote/stop' ] && break
      [ \"\$rc\" -eq 0 ] || exit \"\$rc\"
    done
  " >"$artifacts/$name-screenrecord.log" 2>&1 &
  record_pids+=("$!")
  record_serials+=("$serial")
  record_dirs+=("$remote")
  record_names+=("$name")
  adb -s "$serial" shell "
    for attempt in 1 2 3 4 5 6 7 8 9 10; do
      [ -s '$remote/pid' ] && exit 0
      sleep 1
    done
    exit 1
  " || { echo "call-e2e: screen recording failed to start on $serial" >&2; return 1; }
}

finish_recordings() {
  local i rc=0 remote destination
  for i in "${!record_pids[@]}"; do
    remote="${record_dirs[$i]}"
    adb -s "${record_serials[$i]}" shell "
      mkdir -p '$remote' && touch '$remote/stop'
      if [ -f '$remote/pid' ]; then kill -INT \"\$(cat '$remote/pid')\" 2>/dev/null || true; fi
    " || rc=1
  done
  for i in "${!record_pids[@]}"; do
    remote="${record_dirs[$i]}"
    destination="$artifacts/${record_names[$i]}-screenrecord"
    wait "${record_pids[$i]}" || rc=1
    if adb -s "${record_serials[$i]}" pull "$remote" "$destination" >/dev/null; then
      adb -s "${record_serials[$i]}" shell rm -rf "$remote" || rc=1
    else
      echo "call-e2e: recording remains on ${record_serials[$i]} at $remote" >&2
      rc=1
    fi
  done
  record_pids=()
  return "$rc"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ "$existing_stack" != 1 ]]; then
  echo "== acquire stack slot $slot"
  "$HOME/wonton-dev/stack-slot.sh" acquire --project "$project" "$slot" -- \
    "$dev" up
  own_stack=1
fi
if [[ "$reuse_plugin" == 1 ]]; then
  check_installed_plugin
else
  install_plugin
fi
build_apk
install_apk


for serial in "$serial_a" "$serial_b"; do
  adb -s "$serial" shell am force-stop app.wonton.android >/dev/null 2>&1 || true
  adb -s "$serial" shell monkey -p app.wonton.android 1 >/dev/null
  sleep 2
done

record_screen "$serial_a" a
record_screen "$serial_b" b
launch_widget "$serial_a"
launch_widget "$serial_b"
forward_a="$(forward_webview "$serial_a")"
forward_b="$(forward_webview "$serial_b")"

echo "== join and sample inbound RTP ($duration seconds)"
python3 "$cdp" "$forward_a" join "$duration" >"$artifacts/a-stats.jsonl" &
join_a=$!
python3 "$cdp" "$forward_b" join "$duration" >"$artifacts/b-stats.jsonl" &
join_b=$!
wait "$join_a"
wait "$join_b"
check_stats "$artifacts/a-stats.jsonl"
check_stats "$artifacts/b-stats.jsonl"

cp "$artifacts/a-stats.jsonl" "$artifacts/inbound-rtp-a.jsonl"
cp "$artifacts/b-stats.jsonl" "$artifacts/inbound-rtp-b.jsonl"


echo "== hang up one side"
python3 "$cdp" "$forward_a" hangup 5 >"$artifacts/hangup-a.jsonl"

if [[ "${CALL_SKIP_KILL:-0}" != 1 ]]; then
  echo "== rejoin and force-stop one side"
  launch_widget "$serial_a"
  launch_widget "$serial_b"
  python3 "$cdp" "$forward_a" join 60 >"$artifacts/rejoin-a.jsonl" &
  rejoin_a=$!
  python3 "$cdp" "$forward_b" join 60 >"$artifacts/rejoin-b.jsonl" &
  rejoin_b=$!
  wait "$rejoin_a"
  wait "$rejoin_b"
  adb -s "$serial_a" shell am force-stop app.wonton.android
  python3 "$cdp" "$forward_b" wait-left "${CALL_DELAYED_LEAVE_TIMEOUT:-90}" >"$artifacts/delayed-leave-b.jsonl"
fi


cat >"$artifacts/metadata.json" <<EOF
{
  "mode": "$mode",
  "project": "$project",
  "domain": "$WONTON_DEV_DOMAIN",
  "room": "$room",
  "duration_seconds": $duration,
  "apk": "$(basename "$apk")",
  "plugin": "app.element-call",
  "inbound_rtp_checked": true,
  "delayed_leave_attempted": $([[ "${CALL_SKIP_KILL:-0}" == 1 ]] && echo false || echo true)
}
EOF

echo "Inbound audio/video RTP samples found on both devices; hangup was requested."
echo "Verify the selected ICE transport, tunnel path and participant departure independently."
}

main "$@"
exit
