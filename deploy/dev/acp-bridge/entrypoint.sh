#!/bin/sh
set -eu
umask 077
bridge_data="${WONTON_BRIDGE_DATA:-/data/bridge}"
mkdir -p "$bridge_data" "$(dirname "${WONTON_ACP_SOCKET:-/run/wonton/acp.sock}")"
chmod 700 "$bridge_data"
exec /usr/local/bin/wonton-acp-bridge "$@"
