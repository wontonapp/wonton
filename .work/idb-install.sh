#!/bin/bash
set -x
export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1
export https_proxy=http://127.0.0.1:7899 http_proxy=http://127.0.0.1:7899 all_proxy=http://127.0.0.1:7899
brew install facebook/fb/idb-companion 2>&1 | tail -15
/opt/homebrew/bin/python3 -m pip install --user --break-system-packages fb-idb 2>&1 | tail -4
ls -la /opt/homebrew/bin/idb_companion /opt/homebrew/bin/idb 2>&1 | tail -3
