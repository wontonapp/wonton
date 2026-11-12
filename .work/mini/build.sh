#!/bin/bash
set -eu
cd ~/wonton-dev/link-survey-src
[ -d .git ] || { git init -q && git add -A >/dev/null && git -c user.email=x@x -c user.name=x commit -qm src; }
export PATH=$HOME/.cargo/bin:$PATH CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 WONTON_IOS_TARGETS=aarch64-apple-ios-sim
echo "core start $(date +%T)"
nice -n 10 apps/ios/scripts/build-core.sh debug > ~/wonton-dev/link-survey-logs/core.log 2>&1
rm -rf target
xcodegen generate --spec apps/ios/project.yml > ~/wonton-dev/link-survey-logs/xcodegen.log 2>&1
echo "app start $(date +%T)"
nice -n 10 xcodebuild -project apps/ios/Wonton.xcodeproj -scheme Wonton -configuration Debug \
  -destination 'generic/platform=iOS Simulator' ARCHS=arm64 ONLY_ACTIVE_ARCH=NO \
  -derivedDataPath ~/wonton-dev/link-survey-dd CODE_SIGNING_ALLOWED=NO build > ~/wonton-dev/link-survey-logs/xcode.log 2>&1
app=$(find ~/wonton-dev/link-survey-dd/Build/Products -name 'Wonton.app' -maxdepth 3 | head -1)
rm -rf ~/wonton-dev/link-survey-app && mkdir -p ~/wonton-dev/link-survey-app && cp -R "$app" ~/wonton-dev/link-survey-app/
rm -rf ~/wonton-dev/link-survey-dd
echo "done $(date +%T)"
df -h / | tail -1
