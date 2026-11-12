#!/bin/bash
xcrun simctl terminate F6EF9B7F-9F73-4933-9085-A5CEED038838 app.wonton.ios >/dev/null 2>&1
date +%T
SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="wonton://pair?v=0&id=27ecb5de6ac461e42ca142a81fbac0122053ad61834561f54b25c9d348106522&relays=%5B%7B%22url%22%3A%22https%3A%2F%2Faps1-1.relay.n0.iroh.link.%2F%22%2C%22tier%22%3A0%7D%2C%7B%22url%22%3A%22https%3A%2F%2Feuc1-1.relay.n0.iroh.link.%2F%22%2C%22tier%22%3A0%7D%2C%7B%22url%22%3A%22https%3A%2F%2Fuse1-1.relay.n0.iroh.link.%2F%22%2C%22tier%22%3A0%7D%2C%7B%22url%22%3A%22https%3A%2F%2Fusw1-1.relay.n0.iroh.link.%2F%22%2C%22tier%22%3A0%7D%5D&addr=100.83.35.101%3A19682&domain=100.83.35.101.nip.io&core=http%3A%2F%2F100.83.35.101.nip.io%3A19680%2F&secret=982c8ca262efa90648e76cd7d9b818a8" xcrun simctl launch F6EF9B7F-9F73-4933-9085-A5CEED038838 app.wonton.ios
echo rc=$?
date +%T
