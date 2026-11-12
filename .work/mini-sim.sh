#!/bin/bash
set -eu
udid=$(xcrun simctl create linksv-17pro "iPhone 17 Pro" "iOS 26.5")
echo "UDID $udid"
xcrun simctl boot "$udid"
xcrun simctl bootstatus "$udid" -b
xcrun simctl install "$udid" ~/wonton-dev/link-survey-app/Wonton.app
echo installed
