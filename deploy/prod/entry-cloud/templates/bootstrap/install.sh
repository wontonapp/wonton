#!/bin/sh
# Served at https://names.<domain>/install.sh. The public upstream installer
# stays domain-free; this operator wrapper is the only place the address lives.
set -eu
curl -fsSL "https://@NAMES_HOST@/relay-ca.pem" -o /run/wonton-entry-relay-ca.pem
export WONTON_CLIENT_SERVICE="@PUBLIC_SERVICE_URL@"
export WONTON_ACME_DIRECTORY=https://acme-v02.api.letsencrypt.org/directory
export WONTON_ACME_ACCEPT_TERMS=true
export WONTON_FRP_CA_FILE=/run/wonton-entry-relay-ca.pem
exec sh -c "$(curl -fsSL https://raw.githubusercontent.com/wontonapp/wonton-install/main/install.sh)"
