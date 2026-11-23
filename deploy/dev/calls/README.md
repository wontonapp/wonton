# 通话端到端冒烟（ADR-0096）

在开发栈上把 Element Call 的三种媒体路径各跑一遍，并**按退出码判定**：

- 组 `a`：UDP 直连（两端的选中候选对都是 udp，且 local 不是 relay）。
- 组 `b`：插件只剩 TCP 7881。
- 组 `c`：强制 n0 中继（两台设备上 iptables 丢掉到私网候选的包，再看 App 隧道日志的实际路径是 relay）。
- 组 `kill`：B 正常挂断必须清掉成员；A 强杀后等 60 秒，延迟 leave 没发出时只记录（当前 lk-jwt 0.10.0 不发，见 ADR）。

每组还要求：两端本轮各写一条新的 `call.member`、双向音频和双向视频的收发字节都大于 0（相机和麦克风必须真开）、在网页里点挂断后本轮两条成员的内容清空（Element Call 不删 state 键）。任何一条不成立都以非零退出。

取代 `deploy/dev/core/call-e2e.sh`（已删）：它需要在已配好的两台设备上手工准备，三个路径的判据只在日志里、没有媒体和挂断断言。

## 前置

- 两台 Android 模拟器/设备装好本仓库的 **Debug APK**，`HOST` 上有 adb server，串号分别放 `DEV_A`/`DEV_B`。
- 一台能起开发栈的 `HOST`，上面有 `~/wonton-dev/remote-dev.sh`。栈要先 `remote-dev.sh <名字> up`（同一台机器时 `HOST=localhost` 也行），脚本从 `~/wonton-dev/stacks/dev-<栈>/.dev-ports` 读网关和隧道端口。
- 栈里已经装好通话插件（默认 UDP 版），脚本会就地改外部的插件副本再重装，不会碰插件仓库本身。
- `HOST` 上的 python3 要有 `pyjwt` 和 `cryptography`（矩阵操作要签 EdDSA JWT；`python3 -m pip install pyjwt cryptography`）。`websocket-client` 已随仓库放在 `pylib/`（1.8.0，Apache-2.0，见 `pylib/LICENSE.websocket-client`），脚本推到栈副本后带 `PYTHONPATH` 用。
- 两台设备当前要能互不干扰地跑同一个 App；模拟器同时最多开两台时先关掉别的。

**不碰真实的家庭群**：脚本用 Tuwunel 管理密钥在开发栈里自己建加密房间。

## 插件包

`PKG` 指向插件仓库 `wontonapp/wonton-plugin-call` 的固定提交，拷到 worktree 的 `deploy/dev/calls/plugin/`（已进 `.gitignore`）：

```bash
git clone https://github.com/wontonapp/wonton-plugin-call /tmp/wonton-plugin-call
git -C /tmp/wonton-plugin-call checkout <固定提交>
cp -R /tmp/wonton-plugin-call deploy/dev/calls/plugin
```

## 跑

```bash
STACK=call-smoke HOST=192.0.2.10 DEV_A=emulator-5560 DEV_B=emulator-5562 \
  EVIDENCE=$HOME/wonton-dev/research/call-smoke deploy/dev/calls/smoke.sh
```

- `STAGE=udp|tcp|relay|kill` 只跑一组；默认 `all` 按 udp → kill → tcp → relay 走完。
- `ROOM_ID`/`ROOM_NAME` 复用房间（脚本只比对**本轮新增**的 `call.member`，旧成员不影响判定）；不给就新建。
- `PKG` 改插件包相对路径。
- 同一 `EVIDENCE` 同时只允许一份在跑（`.lock`）。

证据都写进 `EVIDENCE`：`run.log`（判定原文）、`webrtc-*.jsonl`（15 个采样，判定看最大值）、`group-*.png`/`group-*.mp4`、`permissions.log`、`members.log`、`pair-*.raw`。退出码：0 全部通过；1 有组失败或前置失败。

## 已知环境问题

- `remote-dev.sh restart core` 可能丢 compose 的网络别名，核心一直起不来、健康检查超时。组 `c` 会等网关、隧道、Tuwunel 三者都通才继续，180 秒不通直接失败并指到这里。
- `HOST=localhost`（本机模拟器）的配对码用模拟器到宿主机的 `10.0.2.2`；远端主机用它的局域网地址。两种都用 `adb reverse` 打底。
- APK 只编 `arm64-v8a` 时 x86_64 模拟器装不上；模拟器要用 `./gradlew :androidApp:assembleDebug -Pwonton.abis=x86_64`。
- 每轮开始会 `pm clear` 两个设备（配对、权限、本地缓存的房间都清空），再重新配对；这是为了让“全新安装会先出 Loading/授权页”不再被误判成已配对。
- 中继组要 `adb root` 后改设备的 iptables（只动模拟器内部，脚本退出时 `iptables -F OUTPUT` 还原）；真机需要 root/adb root。
