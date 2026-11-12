# feat(android): 插件详情页加“映射到本机”，映射开着时起前台服务

Refs #795（第 4 步：Android 壳）。按 ADR-0072。

## 改了什么、为什么

- **插件详情页的“映射到本机”一组**（`PortMappingSection.kt`，挂在 `InstalledDetailSections` 里，插件声明了
  `tunnel.ports` 时出现）：一张设计系统的分组卡片（`WontonGroup`），开关一行、每个端口一行
  （`127.0.0.1:8080 · TCP/UDP`，`WontonSeparator` 分隔），和“固定到标签栏”一组的写法一致；映射没成功时按核心给的
  类别如实写一句原因（端口被占用、服务器太旧、隧道没连上、插件没装、没声明端口、没在运行、连不上插件，其他类别归为
  “映射出错了”），提示用现有的问题提示组件放在卡片下面。核心没给映射状态时（太旧或还没实现），端口按清单声明的
  列出来，开关按关显示。文案键的映射是纯函数 `portMappingReasonRes`，有单元测试；不写快照和文案断言。
- **前台服务 `PortMappingService`**（`specialUse`，manifest 带 `PROPERTY_SPECIAL_USE_FGS_SUBTYPE`）：有插件开着
  映射时由 MainActivity 起、全关时停，通知栏常驻“正在映射 <应用名> 的端口”；几个应用同时开着时一条概括文案
  （个数）。通知标题在服务里跟着插件列表刷新（服务自己订阅同一份 ViewModel，多处同时收没有影响）。
- **文案**进 strings.xml，简中、英文、繁中、日文四种齐全；`scripts/check-ui-text.py` 通过。

**为什么和“保持连接”分成两个前台服务**：两者的启停条件来自两个不同的 ViewModel（保持连接是全局设置
`home.keep_connection`，映射是每个插件自己的开关 `port_mappings`）。合成一个服务的话，通知内容要靠两个不相关
开关的组合来挑，还得改 ADR-0003 里“启停只看 keep_connection、服务自己不判断”的契约；两个前台服务并存是
Android 允许的，各报各的原因，跟媒体播放服务“互不启停、通知各管各的”的既有约定一致。通知渠道也没有共用：
渠道名显示在系统的通知设置里，“保持连接”盖不住“端口映射”的语义，分开两个渠道主人才能只静掉其中一个。

## 验证

- `nice -n 19 ~/wonton-dev/android-build.sh`（detekt、`:androidApp:testDebugUnitTest`、`:androidApp:assembleDebug`，同 CI android.yml）：**BUILD SUCCESSFUL**；测试结果里 `PortMappingReasonTest` 2/2、`StepCardFrameTest` 1/1，0 失败。
- `python3 scripts/check-ui-text.py`：界面文字检查通过（四种语言的键齐平、占位符一致）。
- **模拟器走查**（npchy 本机模拟器 + 61 的 dev 栈；APK 临时叠在 `origin/feat/tunnel-udp-client` 上编，只用于走查；栈装了声明 `tunnel.ports` 的 whoami 改包）。截图在 `~/wonton-dev/fs-shots/udpmap-android/`：
  - 详情页出现“映射到本机”一组：开关加两行端口 `127.0.0.1:8080 · TCP/UDP`（028-detail-clean.png）；
  - 打开开关 → `dumpsys activity services`：`PortMappingService isForeground=true foregroundId=3 types=0x40000000`（specialUse）、`channel=port_mapping`；通知栏出现“Mapping ports for Who am I”，与“保持连接”通知并存、渠道分开（031-shade.png）；
  - 本机连一下 `127.0.0.1:8080` 触发开流，服务器（服务端未实现，#795 第 2 步未合）回 `UnsupportedRequest` → 一组里如实显示“Your home server is too old and needs an update.”（032-tooold.png）；
  - 关掉开关 → 原因行清掉、`PortMappingService` 从运行服务里消失、通知同步消失（033-toggle-off.png）。
- **审阅意见后的分组卡片改法复验**（1k 模拟器 + 61 的 dev 栈，同一套叠分支 APK）：一组改用 `WontonGroup`
  分组卡片、开关和端口各一行（034-card-off.png），开着 + “服务器太旧”提示在卡片下面（035-card-tooold.png）；
  `android-build.sh` 再次全过（BUILD SUCCESSFUL）。

## 没做的和原因

- 状态 `On` 的实拍（映射真正打通转发）没验证：核心那边的服务端（#795 第 2 步）和客户端（#851）都还没合，走查里
  能看到的是核心回 `UnsupportedRequest` 后的“服务器太旧”类别和开关、通知的启停。`PortInUse` 等其余类别只由
  单元测试覆盖到键映射，没有在模拟器上逐个摆出来（要占端口、停插件这些专门场景）。
- 简中、繁中、日文的界面截图没拍（走查机是英文环境）；四种语言文件的键齐平和占位符一致由
  `scripts/check-ui-text.py` 保证。
- 自动映射（#828）不在这条线里。

## 底座的问题

1. **核心重启后已装插件的 `tunnel_ports` 变空。** 装了带 `tunnel.ports` 的插件后 `dev.sh down`（不清数据）→
   `up`，`GET /_wonton/api/plugins` 里这个插件的 `tunnel_ports` 是 `[]`，详情页因此不显示映射组；卸载重装后才
   恢复。发现于本任务走查（App 是新核心、栈是重启过的旧核心）。建议核心重启时重读插件清单，或确认这是已知行为。
2. **remote-dev 栈配对要宣告地址（WONTON_DEV_ANNOUNCE_ADDRESSES）。** 模拟器和栈不同机时，栈只在 61 上广播
   `192.168.0.61:19182`，模拟器经 npchy 直连不到（无 ICMP 但也不通，等于被丢），n0 公共中继又不可达，App 配对
   一直“Pairing…”不报错（见第 4 条）。把栈的 compose 里 `WONTON_DEV_ANNOUNCE_ADDRESSES` 设成 `10.0.2.2`
   （同机模拟器的地址）加一台 npchy 上的 socat UDP 转发才走通。建议 `remote-dev.sh` 支持给核心传这个变量，或文档
   写明“模拟器 + 远端栈”的这个组合。
3. **配对连不上时不报错。** 配对码里的直连地址不可达、中继也都不可达时，App 的“Pairing…”挂了 4 分多钟不报
   `TransportUnavailable`（`PAIR_TIMEOUT` 是 30 s），只能杀进程。现象在模拟器（无出网）上稳定出现，客户端线
   （#851）可以顺带查一下超时为什么没有生效。
4. **配对码经 adb 传参要加引号。** 码里全是 `&`，`adb shell am start --es wonton_pairing_code <码>` 不加引号会被
   设备 shell 截断，App 报“invalid server ID in the pairing code: missing id”（MainActivity 里的示例命令已带引
   号，容易被当成只是排版）。
