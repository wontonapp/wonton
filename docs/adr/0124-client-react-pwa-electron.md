# ADR-0124 客户端渲染层统一 React：PWA 先做，Electron 后做，核心仍是厚 Rust

- 状态：已接受（2026-11-26 主人拍板；opus-rv-core-1126 复审通过 f71164d5 后定稿）。P1 指第一期：M2 的 PWA 内部预览（团队 + 愿意试的家人）；未定项见文末。
- 部分取代：[ADR-0005](0005-native-clients-thick-core.md) 的客户端 UI 选型（桌面 Compose Multiplatform、Web 的 Compose 编译到 Web）、
  [ADR-0029](0029-platform-material.md) 的“每个客户端都套 iOS/Android 平台材质”、
  [ADR-0120](0120-dumb-shell-pages.md) 的“每种组件每个壳各实现一次”。各自保留什么见“和现有 ADR 的关系”。
- 关联：[ADR-0081](0081-local-agent-host.md)（Electron 包里的本机 Agent 主机和本机服务）、
  [ADR-0033](0033-plugin-tabs.md)（插件网页仍在 WebView）、[ADR-0043](0043-app-languages.md)（界面文字）、
  [ADR-0026](0026-design-system.md)（令牌）、[ADR-0062](0062-desktop-vm-host.md)（Mac/Windows 宿主虚拟机）。
- 原型：11-07 的 Electron + React 壳（分支 `scratch/desktop-proto`，原 ADR-0068，未合入 main）验证了“核心旁路进程 + 帧 + 只渲染 ViewModel”的形态，
  仍然成立的部分见“原型”；分支已删，结论已并入，不再引用本机路径。

## 一句话

浏览器和桌面用同一套 React 渲染层，核心仍是厚 Rust：Web 上核心跑在服务器端的每用户进程里，Electron 里核心跑在本机旁路进程里；
界面只渲染 ViewModel、只发 Action，不碰网络、存储和业务逻辑。

## 背景

主人 2026-11-26 早会定了四件事：

1. 客户端技术栈固定 React。
2. 第一期先做 PWA：main 一合并就自动部署，浏览器打开就是最新版，不用总打包；方便实时预览和调试，像 Agent 应用那样在网页里选元素标注。
3. 桌面壳用 Electron + React，桌面端以后做，形态像 Alma：不只是界面，还带装在这台电脑上的服务，方便手机远程控制这台电脑。
4. Electron 三平台一起，第一期只做 PWA；GOAL、ARCHITECTURE 和 ADR 跟着改。

[ADR-0005](0005-native-clients-thick-core.md) 定的是“iOS SwiftUI、Android Compose、桌面 Compose Multiplatform，维护两套 UI”，
Web 则计划在 M3 把 Compose 编译到 Web（Kotlin/Wasm），核心在服务器上、浏览器经 WebSocket 收发。
[ADR-0029](0029-platform-material.md) 按平台画材质的前提只在 iOS/Android 成立。
[ADR-0120](0120-dumb-shell-pages.md) 想让核心发页面描述、每个壳把每种组件各实现一次；渲染层一旦统一，壳的数量和实现归属都变了。

这份 ADR 只定渲染层、核心放在哪、预览通道和它与旧 ADR 的关系；这期不动原生移动端，去留另定；不定聊天界面和远程控制的协议。

## 主人已定（2026-11-26）

- 核心放服务器端、每用户一份，不编译到 wasm。
- iOS 的 PWA 推送 P1 就做，要求用户把 PWA 添加到主屏幕。
- 设计沿用移动端令牌，但桌面不照搬手机的交互和比例，按桌面重做。
- PWA 网页文件放我们的中继机上，所有人一份、main 合并即更新；这就是预览通道，不用 Cloudflare。
- 浏览器经我们的中继连到家里服务器，按用户限流防止占满带宽；第一期网页版全走中继，以后可用 WebRTC 打洞；手机 App 照旧先打洞。
- Electron 第一版只带客户端；本机 Agent 主机和被控端第一版不带，以后做时默认关。

## 决策

### 1. 渲染层统一 React，界面不做业务

- Web 和 Electron 共用一套 React + TypeScript 渲染层，未来落在 `apps/web`（PWA）和 `apps/desktop`（Electron 壳），
  共用 `apps/web` 的界面代码。本期不动原生移动端（`apps/ios`、`apps/compose`），去留另定。
- 渲染层只有三件事：把 ViewModel 画出来、把用户操作发回去、调用壳提供的平台能力（窗口、剪贴板、选择文件、通知权限）。
  网络、Matrix、iroh、SQLite、推送登记、重试和业务状态全在 Rust 核心里。
- 所有 ViewModel 和 Action 类型来自 `crates/wonton-protocol`，渲染层的 TypeScript 类型由 Rust 生成，不手写第二份。
  帧协议本身也在 Rust 侧定义和测试。
- 渲染层允许保留的本地状态只有：滚动位置、输入框里正在合成的文字、选择与焦点、窗口尺寸、还没被核心确认的乐观回显。
  房间列表缓存、未读数、重试逻辑不算本地状态。

### 2. 核心仍是厚 Rust，Web 和 Electron 只换核心的位置

- 原生移动端继续在进程内用 UniFFI 嵌核心，接口仍是 `dispatch(Action)` 和 ViewModel 流。
- Web 和 Electron 用同一个 `wonton-client-host` 二进制：它链接 `crates/wonton-client`，加一层换行分隔的 JSON 帧，两种传输：
  - Electron：Electron 主进程拉起本机子进程，stdin/stdout，一行一帧；渲染进程经 preload 只拿到 `send`、`onMessage`、`onClose`。
  - PWA：服务器端每个用户一个子进程（不是每个窗口一个），浏览器窗口经中继的 WSS 连同一份核心。
- 两种传输的帧完全一样；换传输不动渲染层，也不动核心。
- 浏览器永远拿不到 Matrix 令牌、设备密钥和 SQLite 存储。

### 3. PWA 的核心放在服务器端、每用户一份，不编译到 wasm

主人 11-26 选定的方案，理由和别的做法见“核心放在哪”。具体形态：

- 每个用户在服务器上只有一份网页核心，它是一台独立的 Matrix 设备：核心为它建数据目录、SQLite 存储和 Matrix 登录；同一用户的所有窗口、不同电脑的浏览器都连这份核心，不按窗口复制核心；第一次从网页配对时创建，之后配对只是授权新浏览器连上同一份核心，不新建核心。浏览器只拿到一个 HttpOnly 的授权 Cookie（中继同源）；WSS 另用从这个 Cookie 换来的短期窗口令牌（只放内存）。
- 和 [ADR-0009](0009-identity-pairing-sso.md) 的设备模型偏差：0009 是“每次安装生成密钥对、配对即一台设备”。网页反过来：每个用户只有一台网页核心 Matrix 设备，浏览器不是新设备，而是核心下的一条授权。设备列表分两层：网页核心一条设备，下面挂授权条目（浏览器名、首次配对、最后活跃）；吊销一条只断那个浏览器，吊销核心才删设备、断所有授权和窗口。0009 继续管手机和桌面 App 的设备身份，不改。
- 每个浏览器授权一条独立凭据：Cookie 只代表这一条授权，核心按（授权, 窗口）管理连接；授权可单独吊销（笔记本被偷只吊销它），不影响同一用户别的浏览器。配对、登录、登出都由中继的 HTTP 面转给核心处理（第 5 节）。
- 窗口 id 的作用域是 `(AuthorizationId, 连接序号)`：帧宿主在每条授权下分配、单增不复用，跨浏览器不撞；未知或过期窗口返回 `WindowGone`。
- 核心起一个 `wonton-client-host` 子进程，数据放在 `core/home/<用户名>/web/`；中继校验 Cookie、Origin 和 CSRF 后，核心只验已认证出站连接上的 HMAC，再把 WSS 升级转到这个进程的本地套接字。每个窗口一个连接、一个 `WindowId`，只带自己的导航栈和当前页等视图状态；聊天、房间、未读、设置这些共享数据只有一份。
- 网页核心在设备列表里可见、可吊销；吊销核心时杀进程、删设备、所有授权和窗口失效。每用户一份核心，不按会话数封顶；窗口/连接上限和回收时间待拍板。
- 默认停在最近一次 ViewModel 快照上重连；没有本地业务状态，离线时只能看最后一屏和“重连中”。
- P1 做 Web Push（iOS 要求先把 PWA 添加到主屏幕）：按 RFC 8030/8291、VAPID 发系统通知；没添加主屏幕的退化成应用内未读。

### 多窗口

- 每个用户一份核心、所有窗口共用；聊天、房间、未读、设置这些共享数据只有一份，只有导航栈、当前页、媒体提示这类视图状态按窗口。
- 现状：Home/Chat/Unread/timeline 的订阅本来就每次独立，多窗口可各自订阅；页面导航状态是全局一份 `PagesLink`，`PageAction::OpenPage`/`ClosePage` 和 `dispatch` 没有窗口身份，两个窗口会互相顶页面。所以要把导航状态按窗口。
- 决策：`WindowId` 按（授权, 连接序号）分配、单增不复用；`PagesViewModel` 的导航字段拆成每窗口 `PageWindowViewModel`；`PageAction` 和帧带窗口；窗口过期返回 `WindowGone`。原生移动端用固定默认窗口。
- 实施拆分、测试和估时见 [#1111](https://github.com/wontonapp/wonton/issues/1111)。


### 4. Electron 是本地旁路进程，不是网页套壳

- Electron 主进程拉起本机的 `wonton-client-host` 子进程，客户端核心直接走 iroh 隧道，和手机一样。
- 渲染进程开 `sandbox`、`contextIsolation`，关 `nodeIntegration`；preload 只暴露三个函数；sidecar 从 `process.resourcesPath` 启动，
  不经 shell。打包开齐 Electron fuses（关 RunAsNode、NODE_OPTIONS、inspect 参数，开 Cookie 加密、只从 asar 加载、asar 完整性校验），
  安装包和自动更新都签名。
- 包里有三个角色，各自进程、各自隧道身份、各自开关（[ADR-0081](0081-local-agent-host.md)）：客户端、本机 Agent 主机、被控端。Electron 第一版只带客户端；本机 Agent 主机和被控端第一版不带，以后做时默认关。
  React 渲染层只是客户端角色，看不到另外两个身份和令牌；被控端的状态经核心转给界面显示。
- [ADR-0081](0081-local-agent-host.md) 的“电脑被手机远程控制”由被控端实现，协议另写 ADR；那份 ADR 固定它装在同一个包里的位置和独立的身份、开关。
- 第一版只启客户端；托盘常驻，退出就停 sidecar；登录自启可选。

### 5. 预览通道：网页文件放我们的中继机，浏览器经中继回家

- 主人 11-26 定：PWA 的网页文件放我们的中继机上，所有人一份，main 合并即更新；这就是预览通道，不用 Cloudflare。中继只做静态文件托管、`/api/web-auth/*` 转发和 WSS 反向转发，不存账号和消息。
- 浏览器经我们的中继连到家里服务器：中继用配对码里带的不透明 `server_id`（装机时随机生成、不是公网地址）查这个用户在这台服务器上的出站连接，按用户限流；没连上返回 503 ServerOffline，浏览器按 1/2/4…最多 30 秒退避重试，页面显示“服务器不在线”，不排队、不缓存 WSS，防止一台浏览器占满中继带宽。浏览器做不了 UDP 打洞，第一期网页版全走中继；以后才用 WebRTC 打洞。手机 App 照旧先打洞。
- 每台服务器不用自己托管 PWA，也不需要公网入口、端口映射或证书，只要能出网连到中继。
- 家里人还是先和自己的服务器配对：浏览器扫 relay 码（`wonton://pair?v=2&relay=…&server=…&code=…`，见 ADR-0009 的 v=2；没有公网地址）或输入服务器 ID、一次性码，手机上已有的设备批准，核心授权这个浏览器（HttpOnly 授权 Cookie + 内存里的短期 WSS 令牌，不写 localStorage）。没配对的人能打开外壳，但没有核心可连。
- relay 码是 ADR-0009 的 v=2 码型，只带 `relay`、`server`、`code`；SAS 不在码里：浏览器生成一次性 ECDH P-256 密钥，核心回自己的公钥，双方算共享密钥，SAS = HMAC-SHA256(共享密钥, 一次性码) 转十进制取前 6 位（不足补 0）；浏览器显示自己算的 SAS，手机显示核心算的同一值，核对一致才批准。中继只转发公钥和码，偷换密钥会得到两个不同 SAS。0009 的 v=0 直接码不变；0123 的“格式照 0009”指 v=0。
- **中继的信任位置**：中继是新的可信中间人。它终结 TLS、发前端 JS、转发 WSS，能看到网页的 ViewModel 和 Action（房间本来不加密，[ADR-0118](0118-im-three-layers-push-with-body.md)）、能改前端代码、能看到授权 Cookie；看不到 Matrix 令牌、设备密钥和 SQLite，那些都在家里的网页核心。它不存消息和账号；内存里只留路由和连接元数据（重启即失、重连重建），落盘只有 VAPID 私钥、每服务器 HMAC 密钥 K、证书公钥记录和推送订阅，均 0600、不进日志。
- **服务器到中继**：家里核心出站连到中继（TLS 客户端证书 + 核心装机时生成一次的 256 位随机服务器凭据 K；首次配对把 **TLS 公钥证书和 K** 交给中继，TLS 私钥不出核心；证书指纹只用于 TOFU 钉人，和 K 分开），30 秒心跳；出站用 mTLS，不逐帧 HMAC；只有 `/api/web-auth/*` 的 HTTP 请求带 `HMAC(K, server_id + 方向 + 方法 + 时间戳 + 路径 + 请求体)`（±60 秒、窗口内去重）。K 不随每次配对换，泄露才重配对换证书和 K、旧钥 24 小时。中继存证书记录和 K（0600、不进日志）。
- **HTTP 面**：中继只多三个入口：`/`（静态壳）、`/api/web-auth/*`（配对开始、轮询、登出，经出站连接转给核心）、`/ws`（升级后转发）。配对批准后中继签发不透明随机 Cookie，并在内存里建会话→路由；Cookie 只给中继看，核心不验。WSS 升级只靠这条 mTLS 出站连接；HTTP auth 请求带 HMAC。
- **推送**：这个 origin 只有一套 VAPID 密钥，Service Worker 的 scope 是 `/`，订阅由中继存。核心把已按订阅公钥加密的负载 POST 给中继，中继用 VAPID 发给浏览器；用户换服务器时订阅不换，负载带服务器 id，旧服务器停推；iOS 仍要求先添加到主屏幕。
- **版本兼容、灰度和回滚**：一份前端对所有服务器，版本窗口 N-1：前端握手带 `ui_build` 和帧协议版本，核心回自己的版本和支持范围；前端支持当前和上一版核心，超出范围显示“更新/重启”，不静默坏。main 合并先在预览给团队，再按中继的构建 id 百分比放量；坏了一键把 `latest` 切回上一个构建，旧资源不删，Service Worker 不强制老人立刻换。协议破坏性变更在 CI 里用两份金样本（当前/上一版核心）跑同一套前端 fixture。P1 的“内部预览”= 团队 + 愿意试的家人，不保证兼容；1.0 前把版本门做成门槛，正式发布不再随时更新。
- 中继托管的是 HTTPS，浏览器只连中继的 WSS；授权 Cookie HttpOnly、Secure、`SameSite=Strict`。以后服务器同源直供时，纯 HTTP 局域网才退化成 `ws://` + 非 Secure Cookie（浏览器在纯 HTTP 下才存），Origin 校验照做。
- P1 只运营这个中继和静态托管，没有注册和公开服务；中继按用户限流，超限行为待定。

### 6. 安全基线

- 传输只允许 WSS（HTTPS）；第一期浏览器只连中继，家里服务器只要能出网到中继。核心只信中继在已认证出站连接上的 HMAC（只覆盖 HTTP auth 请求，WSS 靠 mTLS），不验浏览器 Cookie；升级时校验 `Origin` 和核心状态。CSP `default-src 'self'`，无第三方脚本；`connect-src` 只到中继。
- 授权 Cookie HttpOnly、Secure、`SameSite=Strict`；以后纯 HTTP 直供时去掉 `Secure`。以后服务器同源直供那档的跨源令牌只放内存；P1 中继同源时刷新重连同一核心，Cookie 丢了才重新配对。
- 渲染层不存 Matrix 令牌；浏览器扩展或 XSS 在页面开着时能替用户点，但拿不走设备和推送密钥；网页核心可随时吊销，中继只断开连接、不删核心数据。
- 插件网页不走 React，仍在独立的 WebView（Electron）或受限 iframe（Web）里，按 [ADR-0033](0033-plugin-tabs.md) 的桥通信。
- Electron 的 sidecar 继承本机用户的网络和文件权限，和原生 App 一样；被控端的屏幕录制、辅助功能授权按 [ADR-0081](0081-local-agent-host.md) 的规则单独申请。

### 7. 文案和设计

- 文案按 [ADR-0043](0043-app-languages.md)：核心给类别和原始值，渲染层按语言查自己的资源。Web/Electron 用和核心同一份 Fluent 目录，
  构建时转成前端资源，不新造一套键。
- 设计令牌仍来自 `design/`，桌面沿用移动端的颜色、间距、字号、圆角、动效，但不照搬手机：密度、栏高、列表行高、窗口控件、快捷键和右键菜单按桌面交互与比例重做；同一件事用桌面上惯用的控件，不把手机界面放大。
  原生移动端按 [ADR-0026](0026-design-system.md)、[ADR-0029](0029-platform-material.md) 用自己的令牌；Web 不假装 iOS 的 Liquid Glass 或 Android 的 Material，窗口控件跟随桌面系统。
- 可访问性按 Web 做：语义 HTML、焦点管理、读屏、IME、键盘操作；插件 iframe 的焦点进出要能回主界面。

## 核心放在哪

| 做法 | PWA 的核心在哪 | 好处 | 代价 | 结论 |
| --- | --- | --- | --- | --- |
| A. 编译到 wasm，跑在浏览器 | 浏览器标签页 | 不占服务器内存；断网还能操作本地状态；和 Electron 共用核心代码 | matrix-sdk、iroh、SQLite、加密随机数都要移植或换实现；包大、冷启动慢；密钥和存储落在浏览器，扩展和 XSS 能读；iOS 标签页回收、存储清理；等于维护第二套核心运行时 | 以后做，P1 不做 |
| B. 服务器端核心 | 家里的服务器，每个用户一个进程 | 直接复用现有核心和全部网络栈；浏览器不拿密钥；和 Electron 是同一个二进制、同一套帧；多窗口共享同一份数据和网络连接；不需要 wasm 工具链 | 每用户占服务器 CPU/RAM；核心生命周期、唤醒、回收、吊销要写；多窗口视图状态和连接要管；每次操作走一趟网络；离线差 | **主人已选** |
| C. 浏览器加本机 helper | 浏览器旁边的本机进程 | 兼顾本地核心和浏览器渲染 | PWA 在手机上还得回到服务器端核心，helper 只对桌面浏览器有意义；多一条安装路径 | 只作为 Electron 的形态，不解决 PWA |

选 B 的具体理由：

- `crates/wonton-client` 已经是完整客户端核心，直接跑在服务器上就是复用，wasm 是重写一遍它的依赖栈。
- 浏览器不落密钥：设备密钥、Matrix 令牌、SQLite 都在服务器端的网页核心里，吊销网页核心就断掉，和手机被吊销同一条路。
- 延迟在局域网和隧道里都是一次本地 RTT 量级；文本输入、滚动这些高频动作留在渲染层本地，不逐字往返。
- 编译到 wasm 的坑是真实的：matrix-rust-sdk 依赖 tokio、reqwest、SQLite，iroh 依赖 UDP/QUIC，浏览器里要换 WebSocket/WebTransport 实现；
  这是独立的一大摊，不该压在第一个 PWA 上。体积和可行性要在“待验证”里实测，不能拿设计当结论。

切换点：帧契约和 `Transport` 边界不变，如果以后离线成为硬需求，把宿主从服务器进程换成 wasm 模块即可重评；那时另写 ADR。

## 原型

11-07 的桌面原型（`scratch/desktop-proto`，原 ADR-0068）已经跑通过这些，仍成立并沿用：

- 新行分隔的 JSON 帧，`ClientFrame` 是 `call` / `subscribe` / `cancel`，`HostFrame` 是 `ready` / `ok` / `err` / `next` / `end` / `bad_frame`；
  开发时走本机 WebSocket，正式 Electron 走 stdio。PWA 把 WebSocket 换成 WSS。
- 渲染层只依赖 `Transport` 的 `send`、`onMessage`、`onClose`；`useTopic(topic)` 只暴露 `loading` / `ready` / `ended`。
- TypeScript 类型由 Rust 生成（`ts-rs`），渲染层不手写 ViewModel。
- Electron 安全基线就是上面第 4 节那一套；原型用 Playwright 断言过渲染进程没有 `require`、`process`，`window` 上只有三个函数。
- 设计令牌由生成器出 CSS 自定义属性，`--check` 比对。

不沿用的：原型只做 Mac、只做桌面；“桌面用 Compose Multiplatform、Web 在 M3 才做”是旧结论；
原型里 WebSocket 只给开发用，这里 WSS 是 PWA 的生产传输。

## 和现有 ADR 的关系

| ADR | 取代 | 保留 |
| --- | --- | --- |
| [ADR-0005](0005-native-clients-thick-core.md) | “桌面用 Compose Multiplatform、Web 用 Compose 编译到 Web（Kotlin/Wasm）”；“实际维护两套 UI” | 厚核心、`dispatch(Action)` 进、ViewModel 流出的接口；UniFFI 给原生；Web 核心在服务器、浏览器经 WebSocket 收发 ViewModel 和操作 |
| [ADR-0029](0029-platform-material.md) | “所有客户端都按 iOS/Android 平台材质画” | 共用令牌；原生移动端的 iOS Liquid Glass、Android Material、栏和菜单按平台走、动效时长 |
| [ADR-0120](0120-dumb-shell-pages.md) | “每种组件每个壳各实现一次” | `PageView` 契约、组件白名单、页面版本、未知组件不显示、手写页白名单；React `PageRenderer` 成为新的消费方，原生移动各实现一次不变 |
| [ADR-0014](0014-reserved-capabilities.md) | “Web 端（M3）预留” | 实时通话、跨机 Bot 等预留不变 |
| [ADR-0021](0021-third-party-clients.md) | 不复用它的 SNI 入口做网页；网页另起应用层中继 | 0021 的入口/SNI 规则不变；服务器同源直供时回到 0021 |
| [ADR-0081](0081-local-agent-host.md) | 不取代 | 同一个 Electron 包里的三个角色、各自身份和开关；渲染层只是客户端角色 |
| [ADR-0062](0062-desktop-vm-host.md) | 不取代 | Mac/Windows 宿主虚拟机的服务端形态；Electron 是客户端，不是服务器 |

### 契约、提供方、消费方

- 契约：`wonton-protocol` 的 `ClientFrame`/`HostFrame` JSON 帧（带窗口、`ui_build`、协议版本、HMAC 请求签名）和中继 `/api/web-auth/*` 的 HTTP 面。
- 提供方：`wonton-client-host`（PWA 的服务器端每用户进程；Electron 的本机 sidecar）。
- 消费方：React 渲染层（Web/Electron）。
- 契约测试：同一组帧金样本分别走 stdio 和 WSS，`Transport` 换实现时渲染层不改。

### 和 ADR-0021（入口中继只按 SNI 转发）的关系

- 0021 的入口中继只按 SNI 转发、不解密，TLS 在家里终止；网页中继是另一层：第一期浏览器打不了 UDP 打洞，家里服务器也没有公网入口和证书，只能由中继在公网终结浏览器 TLS。代价是中继看得到明文（房间本来不加密）和能改前端；核心不新增公开监听或入口容器，仍不碰公网 TLS、不解析入口协议，只经 mTLS 出站连接收 WSS。中继是独立服务，被攻破也不拿 Docker socket。
- 服务器有公网入口和证书、同源直供时，TLS 回到家里终止，0021 的模型不变；网页中继那时可以退成可选。
- 所以不是复用 0021 的 SNI 通路，而是在它之外加一层只服务网页的应用层中继；入口容器、SNI 和公网转发的规则不改。

### 和 ADR-0123（受管设备）的边界

- [ADR-0123](0123-hubs-and-managed-devices.md) 的受管设备是用户电脑上的守护进程，替人执行任务；本篇的 Electron 包是客户端渲染层，本机 Agent 主机和被控端是同一包里另外两个角色。三者可以同在一台电脑上，但身份和开关各是各的。
- 受管设备不跑 Matrix 客户端；网页核心的 Matrix 设备也不执行 0123 的任务；设备密钥和 Agent 主机身份不是同一把。

## 不做

- P1 不把核心编译到 wasm，不做离线优先。
- 不做 React Native、不做 Capacitor 套原生移动；React 只做 Web DOM 和 Electron 渲染进程。
- 不恢复 Compose Multiplatform 桌面端，不在 `apps/compose` 里加桌面目标。
- 不运营公开账号系统和云端会话；预览是静态文件 + 中继转发，不做公开注册。
- 不在这里定远程控制的屏幕、输入协议；不在 P1 做被控端。
- P1 不改原生移动端的渲染技术，也不把手机 App 改成 PWA（P1 暂定）；长期去留另定。
- 不新造页面 DSL；声明式页面仍按 [ADR-0120](0120-dumb-shell-pages.md)，聊天、相册、播放器等继续手写。

## 代价与风险

- **服务器资源**：每个用户一个核心进程和一份 SQLite 存储；同一用户开多个窗口不复制核心，只多连接和每窗口的视图状态。五人家庭、每人多台设备的并发用户数会上去；靠挂起和回收压住，默认值要实测。
- **生命周期**：核心进程要处理崩溃重启、核心升级、服务器重启、吊销时的清理；这些比手机上的本地核心多一套监督逻辑。吊销单个浏览器只断它的窗口，吊销核心才断掉这个用户的所有窗口和浏览器。
- **延迟**：每个操作一次网络往返。高频状态留在渲染层，但网络差时按钮和列表会有可感延迟，UI 要如实显示进行中。
- **离线**：服务器连不上时只能看最后一屏；比原生 App 的本地库弱。这是选 B 的主要代价。
- **浏览器侧攻击面**：页面开着时 XSS 或扩展能以用户身份驱动网页核心；HttpOnly 挡不住“代替点击”。CSP、依赖锁定、核心吊销和短期令牌是缓解，不是根除。
- **第三个 UI 技术**：SwiftUI、Compose 之外多一套 React；好处是 Web/Electron 共用一套，坏处是团队要同时维护三种渲染栈。
- **中继单点和带宽**：所有网页连接都走我们的中继，带宽是公共成本，单台中继挂了网页开不了；靠按用户限流压住，限流值、超限行为和扩容待定。
- **中继或服务器离线**：中继 503 时浏览器退避重试并显示“服务器不在线”；配对本身要求服务器在线，不能离线排队。
- **iOS PWA 限制**：Web Push 只在“添加到主屏幕”的 Web App 里可用（iOS 16.4 起），后台同步、存储配额都受限；推送体验达不到原生。
- **Electron 的本机服务**：旁路进程以后带上被控端会扩大本机攻击面；签名、fuses、第一版不带、逐项授权是底线。
- **依赖和供应链**：React、Electron 和前端构建链更新快；锁文件、许可证审查、CSP、自动更新签名都要跟上。
- **预览依赖我们自己的中继机**：不做 Cloudflare，机器、带宽和 DDoS 要自己扛；回滚只回滚网页产物，中继挂了网页本身也打不开。

## 待验证

- 浏览器核心的 WSS 延迟、重连、同一用户多窗口/多标签（同一核心多连接、各自订阅）和慢网络表现；每用户的实际内存。
- 核心挂起、恢复、吊销、重启后的窗口恢复；10 个用户同时在线和同一用户 10 个窗口的负载。
- iOS 主屏幕 Web App 的 Web Push、存储清理、后台切换；Android Chrome 的安装和推送。
- 中继 WSS 的延迟、断线、重连和按用户限流的实际表现；家里服务器出网到中继的稳定性。
- 中继的配对、授权令牌交换和限流；网页产物回滚到上一个版本。
- 中继机的容量、限流和成本；备选/多机扩容方式。
- 服务器同源直供和 WebRTC 打洞的时机（P1 不做）。
- wasm 核心的体积、冷启动和 matrix-sdk/iroh 移植成本，决定以后要不要做。

## 待主人拍板

1. 中继按用户限流的数值、超限行为（拒绝、排队还是降速）和成本口径；单台中继的容量与扩容方式。
2. 每用户窗口/连接上限（不再按会话数封顶）、挂起和回收时间（推荐无连接 15 分钟挂起、24 小时回收）。
3. 以后服务器同源直供和 WebRTC 打洞的时机（P1 全走中继）。

## 出处

- Web Push：<https://www.rfc-editor.org/rfc/rfc8291>、VAPID <https://www.rfc-editor.org/rfc/rfc8292>。
- Electron 安全基线：<https://www.electronjs.org/docs/latest/tutorial/security>、<https://www.electronjs.org/docs/latest/tutorial/fuses>。
- iOS 主屏幕 Web App 的 Web Push：<https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/>。
- Service Worker 需要安全上下文：<https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API>。
- 现有客户端核心接口：`crates/wonton-client/src/lib.rs`、`crates/wonton-protocol/src/action.rs`。
