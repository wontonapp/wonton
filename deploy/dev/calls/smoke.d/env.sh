#!/usr/bin/env bash
# 通话冒烟：必需环境变量、默认值和证据目录；由 smoke.sh 在开头 source。
# shellcheck shell=bash
# shellcheck disable=SC2034 # 这两个变量给同一轮 source 的别的库用，单文件分析看不见赋值。

: "${STACK:?}" "${HOST:?}" "${DEV_A:?}" "${DEV_B:?}" "${EVIDENCE:?}"
stage="${STAGE:-all}"
pkg="${PKG:-deploy/dev/calls/plugin}"
mkdir -p "$EVIDENCE"
chmod 700 "$EVIDENCE"

