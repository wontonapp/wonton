#!/bin/bash
export PATH=/opt/homebrew/bin:$PATH
date +%T
xcrun simctl launch F6EF9B7F-9F73-4933-9085-A5CEED038838 app.wonton.ios
echo rc=$?
date +%T
