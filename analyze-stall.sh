#!/bin/bash
# 分析一次失败复现的日志：按时间轴列出隧道和 iroh 路径事件，定位 /after 卡住的 15 秒。
# 用法: analyze-stall.sh <run.log>
LOG="${1:?run.log}"
echo "=== 测试断言/panic ==="
grep -n "panicked\|FAILED\|assert" "$LOG" | head -10
echo "=== 客户端状态机（wonton_tunnel） ==="
grep -n "wonton_tunnel" "$LOG" | head -60
echo "=== iroh 路径事件（open/selected/abandoned/set_status） ==="
grep -n "iroh::_events::path\|iroh::_events::qnt\|iroh::_events::conn" "$LOG" | head -60
echo "=== 连接建立/断开/超时 ==="
grep -n "隧道连接\|现有连接\|建连\|no response\|validation\|idle\|abandon" "$LOG" | head -40
