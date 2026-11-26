#!/bin/sh
# Served at https://names.<domain>/install.sh. The public upstream installer
# stays domain-free; this operator wrapper is the only place the address lives.
set -eu
export WONTON_CLIENT_SERVICE="@PUBLIC_SERVICE_URL@"
exec sh -c "$(curl -fsSL https://raw.githubusercontent.com/wontonapp/wonton-install/main/install.sh)"
