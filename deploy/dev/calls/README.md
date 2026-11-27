# 通话端到端冒烟（ADR-0096）

在开发栈上把 Element Call 的三种媒体路径各跑一遍，并**按退出码判定**：

- 组 `a`：UDP 直连（两端的选中候选对都是 udp，且 local 不是 relay）。
- 组 `b`：插件只剩 TCP 7881。
- 组 `c`：只剩中继（两台设备上 iptables 丢掉到私网候选的包，再看 App 隧道日志的实际路径是 relay）。给 `RELAYS` 时还要求路径落在指定的自建中继上、媒体 20 秒不断，见下节。
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

只有会重装插件的组（`all`、`tcp`）需要插件包；`udp`/`relay`/`kill`/`plain` 不碰它，没放也能跑。

`PKG` 指向插件仓库 `wontonapp/wonton-plugin-call` 的固定提交 `2ee7351`，拷到 worktree 的 `deploy/dev/calls/plugin/`（已进 `.gitignore`，每个新检出都要自己放）：

```bash
git clone https://github.com/wontonapp/wonton-plugin-call /tmp/wonton-plugin-call
git -C /tmp/wonton-plugin-call checkout 2ee7351
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
- `PKG` 改插件包的 worktree 内相对路径（只有 `all`/`tcp` 组用得到，见“插件包”）。
- 同一 `EVIDENCE` 同时只允许一份在跑（`.lock`）。
- 每组有组级 deadline（`GROUP_DEADLINE`，默认 600 秒，从配置完成后起算）：卡在开房间、join、`ensure_media` 任一步，到点就 `fail_group` 出判决并留错因，不被一步卡死吞掉整轮。
- 失败时（relay 组）自动把两端 App 核心日志尾、logcat 关键行（`ERR_CONTENT_LENGTH`、`Failed to connect`、`transport-unavailable`）和核心容器日志尾落成 `failure-*-*.log`；`check_tunnel_relay` 无论过没过都把最近 `CheckedPath` 和错因写 `relay-path-<a|b>.log`。中继组先判路径、再做媒体断言，路径证据不会被媒体卡住拖没。

证据都写进 `EVIDENCE`：`run.log`（判定原文）、`webrtc-*-<组>.jsonl`（15 个采样，判定看最大值；plain 组也是自己的前缀）、`group-<组>-*.png`/`group-<组>-*.mp4`、`permissions.log`、`members.log`、`pair-*.raw`、`relay-path-<a|b>.log`、失败时的 `failure-*-*.log`。退出码：0 全部通过；1 有组失败或前置失败。

## 用自建中继跑

组 `c` 默认只要求“路径是 relay”，n0 也满足。要验自建中继，把中继表交给脚本：它换表、重起核心，断言接口给的正是这张表，再在媒体阶段判路径和连续性：

```bash
RELAYS='https://<令牌>@relay.808cbd.com,no-n0' STACK=call-smoke HOST=localhost \
  DEV_A=emulator-5560 DEV_B=emulator-5562 EVIDENCE=$HOME/wonton-dev/research/call-relay \
  STAGE=relay deploy/dev/calls/smoke.sh
```

- `RELAYS` 是核心 `WONTON_RELAYS` 的原值（准入令牌写在 userinfo）。表里必须写明 `no-n0`（去掉内置 n0）或 `n0`（保留兜底）：不写按核心默认算保留，检查器会提示。脚本在配对前把它写进 worktree 的 `deploy/dev/core/.env`（`WONTON_DEV_RELAYS`，remote-dev 的 `up` 会推到栈上）、重起核心，再核对核心容器的环境变量和 `/_wonton/api/tunnel` 的返回：表项要和 `RELAYS` 逐项相同（地址、令牌、档位），有 `no-n0` 时不能出现 n0，响应要有 `endpoint_id` 和 `direct_addrs`；原文落 `relay-table.json`。跑完还原 `.env`。只换表不想每轮重编核心时，脚本会 touch worktree 根的 `.dev-no-build`（跳过镜像构建）和 `.dev-no-deps`（只 `compose up --no-deps core`，不重跑在共用机上会卡十几分钟的 `hydra-migrate`），退出时清掉 `.dev-no-deps`。
- `RELAY_HOST` 默认从 `RELAYS` 取。`check_tunnel_relay` 要求两端最近的 `CheckedPath` 同时有 `relay: true` 和目标主机；判定行（`设备 a 隧道走中继 relay.808cbd.com：CheckedPath { … }`）进 `run.log` 和 `relay-path-<a|b>.log`，走到 n0 或别的中继都算失败。relay 组在统计后再判一次两端路径；压掉直连后先等两端真的经中继重连再回房间（实测要 30–40 秒）。
- 媒体判定后 `check_relay_continuity` 两端各采 24 秒（每 2 秒一拍）：每拍都要有已连接的候选对，收音频字节逐拍增长且不低于 4 kbit/s（`RELAY_MIN_KBPS` 可改）。速率、丢包、抖动和判据原文都进 `relay-stats-c.log`，采样原件是 `relay-stats-<a|b>.jsonl`。
- 先 `STAGE=udp` 跑通直连确认链路正常，再 `STAGE=relay`。

## 已知环境问题

- `remote-dev.sh restart core` 可能丢 compose 的网络别名，核心一直起不来、健康检查超时。组 `c` 会等网关回 HTTP、Tuwunel 回 `/_matrix/client/versions` 才继续，180 秒不通直接失败并指到这里（不看 `dev.sh status` 的“已连接”字样，它只打容器状态）。
- `HOST=localhost`（本机模拟器）的配对码用模拟器到宿主机的 `10.0.2.2`；远端主机用它的局域网地址。两种都用 `adb reverse` 打底。
- 上一轮被 `kill -9` 时 EXIT trap 不跑，`block_direct` 的 DROP 会留着把下一轮的隧道端口一起挡死；脚本开跑时会先 `iptables -F OUTPUT` 清一次。
- APK 只编 `arm64-v8a` 时 x86_64 模拟器装不上；模拟器要用 `./gradlew :androidApp:assembleDebug -Pwonton.abis=x86_64`。
- EC 的引导层（`Got it`）和媒体权限弹窗是原生层：`adb input tap` 在上面经常不生效（实测点十次没反应），DPAD 才稳——`Got it` 用 `input keyevent 23`，`Allow` 用 `22` 再 `23`。脚本已经先走 DPAD，`tap_text` 只兜底。
- 每轮开始会 `pm clear` 两个设备（配对、权限、本地缓存的房间都清空），再重新配对；这是为了让“全新安装会先出 Loading/授权页”不再被误判成已配对。**relay 组例外**：它故意先在直连下把 EC 页面打开把 JS 包放进缓存，再下规则。
- 自建中继当前跑 iroh-relay v1.4.1；`curl` 裸发 `Sec-WebSocket-Protocol: iroh-relay` 会被 400
  `unsupported relay version`（v1/v2 是 101），而 iroh 1.5.0 客户端实际发 `iroh-relay-v2, iroh-relay-v1`、v1.4.1 也接受，
  所以版本不匹配只是可疑项。relay 组现在卡在 block 后的 `transport-unavailable`/`relay_recv_channel closed`；relay 镜像
  已升 v1.5.0（`deploy/dev/entry-cloud/compose.yaml` 钉了摘要），重部署后再跑。
- 中继组要 `adb root` 后改设备的 iptables（只动模拟器内部，脚本退出时 `iptables -F OUTPUT` 还原）；真机需要 root/adb root。
