#!/bin/bash
/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator -CurrentDeviceUDID F6EF9B7F-9F73-4933-9085-A5CEED038838 > /Users/jiale/wonton-dev/simulator.log 2>&1 &
sleep 5
pgrep -x Simulator || echo NO_SIMULATOR
