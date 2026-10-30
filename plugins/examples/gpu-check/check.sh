#!/bin/sh
# 在容器里确认拿到了显卡：打印身份和设备节点；有 /dev/kfd 就跑 rocminfo，有 nvidia-smi 就跑它。至少一个报出显卡
# 才写 /tmp/gpu-check.ok（健康检查看它），然后一直运行着。rocminfo 开不了设备时以非 0 退出，容器随之退出，
# 安装失败并附上这里的输出。
set -eu
echo "== id"
id
echo "== devices"
ls -la /dev/dri /dev/kfd 2>&1 || true
ok=0
if [ -e /dev/kfd ]; then
  echo "== rocminfo"
  rocminfo
  # CPU 也是 agent，只认 gfx 开头的显卡。
  if rocminfo | grep -q 'gfx'; then ok=1; fi
fi
if command -v nvidia-smi >/dev/null 2>&1; then
  echo "== nvidia-smi"
  nvidia-smi
  ok=1
fi
if [ "$ok" = 1 ]; then
  touch /tmp/gpu-check.ok
  echo "== ok"
else
  echo "== 没有报出可用的显卡" >&2
fi
exec sleep infinity
