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

- `STAGE=udp|tcp|relay|kill|plain` 只跑一组；默认 `all` 按 udp → kill → tcp → relay 走完，**一组失败也继续跑后面的组**，最后按失败组数汇总退出码。`plain` 用不加密房间跑 UDP 组，记录 Element Call 在不加密房间里的行为。
- `relay` 组会先在直连下把房间和 Element Call 页面打开（EC 入口 JS 有 325 KB，公网中继会把约 100 KB 以上的 HTTP 响应流截断成 `ERR_CONTENT_LENGTH_MISMATCH`，冷缓存下 EC 永远起不来），再下 iptables 规则；拔掉直连后**不要重启 App**，让 iroh 自己把路径切到 relay。n0 公共中继会对 endpoint 限速，隧道流会反复 `transport-unavailable`，中继组的媒体能不能通取决于限速；2026-11-25 只到 `CheckedPath { relay: true }` + 两条 `call.member`，LiveKit 没连上。
- 每次跑都从远端 `stack-slot.sh ports <栈名>` 重新读网关和隧道端口（名额重排会换端口，旧 `.dev-ports` 会让配对和网关都连不上）。
- `ROOM_ID`/`ROOM_NAME` 复用房间（脚本只比对**本轮新增**的 `call.member`，旧成员不影响判定）；不给就新建。
- `PKG` 改插件包相对路径。
- 同一 `EVIDENCE` 同时只允许一份在跑（`.lock`）。
- 每组有组级 deadline（`GROUP_DEADLINE`，默认 600 秒，从配置完成后起算）：卡在开房间、join、`ensure_media` 任一步，到点就 `fail_group` 出判决并留错因，不被一步卡死吞掉整轮。
- 失败时（relay 组）自动把两端 App 核心日志尾、logcat 关键行（`ERR_CONTENT_LENGTH`、`Failed to connect`、`transport-unavailable`）和核心容器日志尾落成 `failure-*-*.log`；`check_tunnel_relay` 无论过没过都把最近 `CheckedPath` 和错因写 `relay-path-<a|b>.log`。中继组先判路径、再做媒体断言，路径证据不会被媒体卡住拖没。

证据都写进 `EVIDENCE`：`run.log`（判定原文）、`webrtc-*-<组>.jsonl`（15 个采样，判定看最大值；plain 组也是自己的前缀）、`group-<组>-*.png`/`group-<组>-*.mp4`、`permissions.log`、`members.log`、`pair-*.raw`、`relay-path-<a|b>.log`、失败时的 `failure-*-*.log`。退出码：0 全部通过；1 有组失败或前置失败。

## 已知环境问题

- `remote-dev.sh restart core` 可能丢 compose 的网络别名，核心一直起不来、健康检查超时。组 `c` 会等网关回 HTTP、Tuwunel 回 `/_matrix/client/versions` 才继续，180 秒不通直接失败并指到这里（不看 `dev.sh status` 的“已连接”字样，它只打容器状态）。
- `HOST=localhost`（本机模拟器）的配对码用模拟器到宿主机的 `10.0.2.2`；远端主机用它的局域网地址。两种都用 `adb reverse` 打底。
- 上一轮被 `kill -9` 时 EXIT trap 不跑，`block_direct` 的 DROP 会留着把下一轮的隧道端口一起挡死；脚本开跑时会先 `iptables -F OUTPUT` 清一次。
- APK 只编 `arm64-v8a` 时 x86_64 模拟器装不上；模拟器要用 `./gradlew :androidApp:assembleDebug -Pwonton.abis=x86_64`。
- 每轮开始会 `pm clear` 两个设备（配对、权限、本地缓存的房间都清空），再重新配对；这是为了让“全新安装会先出 Loading/授权页”不再被误判成已配对。**relay 组例外**：它故意先在直连下把 EC 页面打开把 JS 包放进缓存，再下规则。
- 中继组要 `adb root` 后改设备的 iptables（只动模拟器内部，脚本退出时 `iptables -F OUTPUT` 还原）；真机需要 root/adb root。
