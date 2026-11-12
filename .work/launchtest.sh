#!/bin/bash
xcrun simctl list devices | grep linksv
xcrun simctl terminate F6EF9B7F-9F73-4933-9085-A5CEED038838 app.wonton.ios; echo terminate_rc=$?
date +%T
timeout 20 xcrun simctl launch F6EF9B7F-9F73-4933-9085-A5CEED038838 app.wonton.ios; echo launch_rc=$?
date +%T
