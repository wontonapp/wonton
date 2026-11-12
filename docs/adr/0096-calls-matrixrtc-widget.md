# ADR-0096 音视频通话：MatrixRTC + LiveKit，Element Call 作为 Matrix widget 跑在插件页里

- 状态：提议（2026-11-12）。路线（同 Element X：MatrixRTC + LiveKit + Element Call 网页嵌进 App）由主人 2026-11-08 定
- 细化 [ADR-0014](0014-reserved-capabilities.md) 的“实时通话”：只定房间里的音视频通话；和 Bot 通话、屏幕共享、来电响铃不在这篇
- 用 [ADR-0072](0072-tunnel-udp.md) 的 UDP 隧道，和 [ADR-0033](0033-plugin-tabs.md) 的插件页、桥、摄像头和麦克风授权

## 背景

Element X 的通话（一对一也是）全走 MatrixRTC：房间里的 `m.call.member` 状态事件宣告谁在通话里（MSC4143），媒体经 LiveKit
SFU（MSC4195）；客户端把 Element Call 网页当 Matrix widget 嵌进 WebView，用 widget API（MSC2762、MSC3819 to-device、
MSC4157 delayed events、MSC4515 RTC transports）替它收发事件。Element X 从未支持旧的 1:1 `m.call.*`，matrix-rust-sdk 也
没有那套信令。

已经有的积木：

- **隧道**：ADR-0072 把 LiveKit 列为用例，插件声明 `tunnel.ports`，App 在手机本机 `127.0.0.1` 同号映射。2026-11-11 在
  Android WebView 里实测通过（ADR-0072“验证结果”）：UDP 直连、TCP 7881 退路、n0 公共中继三组都连上，中继组首帧 6.3 秒、
  码率贴着约 1 Mbit/s。iOS WKWebView 没验。
- **插件页**：WebView、桥（MCP Apps 的 JSON-RPC，注入脚本接住页面发给自己的 `postMessage` 转给核心）、摄像头和麦克风的
  逐插件授权（ADR-0033“插件网页的摄像头和麦克风”）。
- **matrix-sdk 0.19.1**（`vendor/matrix-sdk`）带 widget 驱动（`experimental-widgets` feature）：`WidgetDriver::new(WidgetSettings)`
  给出驱动和句柄，`run(room, CapabilitiesProvider)` 跑状态机；句柄的 `send` 收页面发来的 JSON，`recv` 出要发给页面的 JSON。
  批准哪些能力由宿主的 `CapabilitiesProvider::acquire_capabilities` 决定；OpenID、delayed events 的代发和续期、加密房间里
  to-device 的加解密、RTC transports 发现都在驱动里。
- **Tuwunel v2.0.0**（开发栈已用）：实现 MSC4140 delayed events（Element Call 靠它在客户端崩溃后自动退出通话）；
  `[global.well_known] livekit_url` 同时出现在 `.well-known/matrix/client` 的 `org.matrix.msc4143.rtc_foci` 和
  `GET /_matrix/client/unstable/org.matrix.msc4143/rtc/transports` 里（v2.0.0 源码 `src/api/client/rtc.rs`、
  `src/core/config/well_known.rs`）；`GET /_matrix/federation/v1/openid/userinfo` 在 `allow_federation = false` 时照样注册
  （`src/api/federation/routes.rs` 的 `register_server_misc_routes`）。实测见“验证结果”。

缺的是两头：一个带 LiveKit 的插件，和让插件页能当 Matrix widget 跑的宿主能力。

## 决策

### 1. 组件和接口

新插件 `wonton-plugin-call`，三个服务都拉上游官方镜像、固定版本，不改源码：

| 服务 | 镜像 | 许可证 | 做什么 |
| --- | --- | --- | --- |
| `livekit` | `livekit/livekit-server` | Apache-2.0 | SFU。`rtc.node_ip: 127.0.0.1`、UDP mux 一个端口、`tcp_port: 7881`、`room.auto_create: false`（只让授权服务建房间） |
| `jwt` | `ghcr.io/element-hq/lk-jwt-service` | AGPL-3.0 | MSC4195 的授权服务：验 OpenID 令牌换 LiveKit 令牌，代管断线后的 delayed leave |
| `web` | `ghcr.io/element-hq/element-call`（完整包） | AGPL-3.0 | Element Call 网页，是插件的 `webui`；同一个入口把 `/livekit/sfu` 反代到 `livekit:7880` 的信令 WebSocket |

- **信令走插件网页，不另开映射。** 页面经 App 的本地代理进网关，信令 WebSocket 和页面同源（试点就是这样连的）。映射只给
  媒体：UDP mux 和 TCP 7881。7880 只在插件网络里，不映射到手机上（ADR-0072：映射出去的端口不能不带认证）。
- **AGPL 的两个只作为独立容器运行**，插件详情页写上游源码地址。App 和核心不打包 Element Call 的 embedded 包：打进 App 就是
  分发 AGPL 代码，每次升级 Element Call 还要发 App。
- **lk-jwt-service 用独立模式**（`/sfu/get`、`/get_token`），不用它推荐的 application service 模式：那要服务器实现
  MSC4502、MSC4512，Tuwunel v2.0.0 没有。独立模式上游已标弃用，见“代价与风险”。

**SFU 怎么被找到：** 走标准的路。Tuwunel 公布 `livekit_url`（MSC4143 的 `rtc_foci` 和 `/rtc/transports`），Element Call
经 widget API 的 `rtc_transports` 能力拿到；matrix-sdk 的 `discover_rtc_transports` 先问服务器的接口，再退到 `.well-known`。

- `livekit_url` 写一个**固定地址**：Matrix 对外地址下的 `/_wonton/matrix-rtc`。Tuwunel 的配置不随装卸插件改，不用为装插件
  重启聊天服务器。
- 网关把这个前缀转给提供 `matrix_rtc` 能力的插件（ADR-0053 的 `provides`；契约是 MSC4195 livekit 授权服务的接口，提供方是
  `wonton-plugin-call`，使用方是任何 MatrixRTC 客户端）。没装提供方或它没在跑时回 503 和明确的错误类别，App 的入口也不显示
  （第 3 节），不装作能打。
- 这个前缀不要 Matrix 访问令牌（授权靠请求体里的 OpenID 令牌），网关不按 Matrix 客户端接口认它。
- 核心部署给 Tuwunel 配 `livekit_url`（`TUWUNEL_WELL_KNOWN__LIVEKIT_URL`，开发栈的 `tuwunel.toml` 同值）；Tuwunel 升到 2.0
  由聊天线做，这篇不安排升级。

**授权服务怎么验 OpenID：** lk-jwt-service 按标准的服务器发现找主人的服务器：取 `https://<服务器名>/.well-known/matrix/server`，
再请求 `https://<委托地址>/_matrix/federation/v1/openid/userinfo`，只走 HTTPS（v0.9.0 `src/helper.rs` 的
`resolve_federation_base_url`）。插件网络里解析不了服务器名，也连不到 Tuwunel，所以：

- 核心给 `matrix_rtc` 的提供方注入 `WONTON_MATRIX_SERVER_NAME`（插件拿它填 `LIVEKIT_FULL_ACCESS_HOMESERVERS`），并在它的网络里
  把服务器名解析到核心（同 ADR-0063 的核心别名）。核心在这个别名上只答：`/.well-known/matrix/server`（指回自己），
  `/_matrix/federation/v1/openid/userinfo`、`/.well-known/matrix/client` 和 MSC4140 不稳定路径上的
  `POST /delayed_events/{delay_id}/{action}`（这三样原样转给 Tuwunel），别的一律 404；也只答 `matrix_rtc` 提供方网络里来的请求。
- 别名是核心的一个 TLS 监听（容器里 443，`WONTON_MATRIX_RTC_LISTEN`）。`.well-known/matrix/server` 指回 `<服务器名>:443`，
  免得授权服务退到 8448。
- **证书：** 核心有一把自己的 CA（私钥在核心的状态目录里，重启不变），给服务器名签别名的证书。装提供方时核心把
  “核心看到的系统根证书 + 这个 CA”写成证书包，放进提供方数据目录的 `matrix-rtc-ca/ca-bundle.pem`，目录经
  `WONTON_MATRIX_CA_DIR` 交给插件；插件挂进去、`SSL_CERT_FILE` 指向它。追加而不是替换：只信核心的 CA 就验不了别的服务器
  来的用户，跳过校验则谁的证书都认。
- **代管 delayed leave：** Element Call 断线后靠 delayed leave 退出通话，lk-jwt-service 可以替它续期和发出。它按服务器名找
  客户端接口，插件设 `LIVEKIT_CS_API_URL_OVERRIDES=<服务器名>=https://<服务器名>` 指回别名（Tuwunel 的
  `.well-known/matrix/client` 没配，转过去是 404），别名把操作原样转给 Tuwunel。Tuwunel v2.0.0 的不稳定路径专门为这类调用方
  免访问令牌（`src/api/client/delayed_events/update_delayed_event.rs`）。

### 2. 核心：插件页可以作为 Matrix widget 跑

核心做的是通用能力：**一个插件页可以声明自己是 Matrix widget，在某个房间的上下文里打开，经桥说 widget API。** 核心不认识
Element Call，也不认识“通话”；判断都按清单声明和 widget 规范来。换一个别的 MatrixRTC 网页或别的 widget，核心不用改。

**清单：**

```yaml
matrix_widget:
  # 网页入口下的路径和查询。占位符用 widget 规范的（$matrix_room_id、$matrix_user_id、$matrix_widget_id、
  # $org.matrix.msc2873.matrix_device_id、$org.matrix.msc4039.matrix_base_url 等），由 matrix-sdk 替换；
  # 另加一个核心替换的 $org.wonton.page_origin（页面自己的 origin，Element Call 的 parentUrl 要它）。
  url: /room?widgetId=$matrix_widget_id&roomId=$matrix_room_id&parentUrl=$org.wonton.page_origin&intent=join_existing
  entry: video_call          # 房间顶栏的图标：video_call、voice_call、other；不认识的按 other
  capabilities:              # MSC2762 的能力串，只给这里列的
    - org.matrix.msc2762.receive.state_event:org.matrix.msc3401.call.member
    - org.matrix.msc2762.send.state_event:org.matrix.msc3401.call.member
    - org.matrix.msc3819.send.to_device:io.element.call.encryption_keys
    - org.matrix.msc4157.send.delayed_event
    - org.matrix.msc4515.rtc_transports
    # ……
```

- 通话专属的参数（`intent`、`header`、`confineToRoom`、`skipLobby` 等）由插件写死在 `url` 里，核心不懂它们。
- 有 `matrix_widget` 就要有 `webui`。和 `webui.cross_origin_isolated` 互斥：声明了跨源隔离的页面在 Android 上交给系统浏览器
  打开（ADR-0033），浏览器里没有桥，widget 跑不起来。清单检查当场报 `invalid_input`。

**能力白名单（核心实现的 `CapabilitiesProvider`）：** 批准的 = 页面请求的 ∩ 清单声明的 ∩ 核心允许的。清单外的不给，按规范
回实际批准的能力，日志记一条。核心允许的：

- 读、发房间事件和状态事件：按事件类型列，不收通配。
- 发状态事件时 `state_key` 只能是自己的用户 ID，或以 `<自己的用户 ID>_`、`_<自己的用户 ID>_` 开头（MSC3779 的“自己的状态”，
  Element Call 的 `m.call.member` 就这样写）。核心批准时按当前登录的账号收窄，页面改不了别人的状态。写成测试。
- to-device：按事件类型列。加密房间里驱动自动加密发送、只转交加密收到的（0.19.1 `widget/matrix.rs`）。
- `send.delayed_event`、`update_delayed_event`、`rtc_transports`、`requires_client`、`download_file`。
- 不给：别的房间（`timeline:` 能力）。页面只拿到打开它的那个房间。
- **OpenID：** 驱动对 `get_openid` 不查能力，直接向服务器要令牌（0.19.1 `widget/machine/mod.rs`）。所以声明了 `matrix_widget`
  就等于能拿到一小时的 OpenID 令牌，令牌只能让对方换回用户 ID。确认页写明。

**主人在哪确认：沿用“声明 + 醒目提示”（ADR-0036、ADR-0057），不在每次打开时问。** 安装和更新的确认页、确认卡片按类别列出
widget 能力：核心给原始能力串和级别（普通、高），确认页的文字由壳按级别写，确认卡片由核心写：

- 普通（不标色）：读写通话、回应这类不含聊天内容的事件；to-device；delayed events；查通话服务器；OpenID。
- 高（同 `network.lan` 的级别）：读或发 `m.room.message`、`m.room.encrypted`、`m.room.redaction`、`m.room.member`、
  `m.room.power_levels` 这类会读到聊天内容、改动历史或成员的类型。提示写明“这个插件的网页能在你打开它的房间里读到聊天内容／
  以你的名义发消息、撤回消息”。Element Call 要撤回自己的举手回应（`m.room.redaction`），所以它是这一级。
- 更新时能力变多，照常重新确认（同 ADR-0033 跨源隔离的规矩）。
- 家人账号（ADR-0088）用的是主人装的插件；widget 以当时登录的账号行事，只碰这个人自己打开的房间。

**桥：** 同一个插件页桥，加第二种报文。注入脚本按 `api` 字段接住页面发给自己的 widget 消息（`fromWidget` 请求、`toWidget`
应答）转给核心，核心交给 `WidgetDriverHandle::send`；`recv` 出来的消息由核心包成合成 `MessageEvent` 的脚本注入回同一个页面。
来源判定、主框架判定、壳不解析报文，都同 ADR-0033。不是以 widget 身份打开的插件页收到 widget 消息照旧忽略。

**会话：** 打开方式是一种新的标签身份“某插件的 widget、某房间”（`TabRef` 加一种）。核心为它建一个驱动、每次打开新生成
widget ID、用 `generate_webview_url` 得出地址。页面关掉、换房间、退出登录时驱动结束；同一时间只有一个 widget 会话。

### 3. 入口

- **发起和加入：** 装了声明 `matrix_widget` 的插件、插件在运行时，房间顶栏出现一个按钮，图标按 `entry`。点了全屏打开这个
  插件的 widget 页，带上这个房间。房间里已经有人在通话（核心读房间的 `m.call.member` 状态）时按钮显示“通话中”，点了就是
  加入。装了几个时出菜单，名字用插件名。
- **来电响铃不在这篇。** 要先有 iOS 推送（APNs，再加 VoIP push 和 CallKit）；Android 照 Element X 用高优先级的来电通知。
  Element Call 照发 MSC4075 的 `m.rtc.notification`，响铃那篇再接。
- **第一版离开通话页就挂断：** 切走、App 进后台时页面结束，Element Call 退出通话（崩溃、断网由 delayed leave 兜住）。后台继续
  通话、画中画要 CallKit 或前台服务，和响铃一起做。

### 4. 网络

- 清单 `tunnel.ports`：`{livekit, <UDP mux 端口>, udp}`、`{livekit, 7881, tcp}`，对齐 ADR-0072。映射在 widget 页可见时开、
  不可见时关，同 ADR-0072 的“App 自己用的”；第一版离开就挂断，用不上“通话进行中另算”。
- **不声明 `cross_origin_isolated`。** Element Call v0.28.2 自带的 nginx 不发 COOP/COEP，网页不要求跨源隔离；声明了反而在
  Android 上交给浏览器、丢了桥（第 2 节）。
- **中继下如实提示：** 映射开着、路径是 n0 公共中继时，沿用 #296 的 `PortMappingWarning::PublicRelayBandwidth`，通话页上方
  一条轻提示“只连上了公共中继，画面可能卡”，不拦着打。

### 5. 两端的壳

权限桥归前端线（已有卡），这篇只定约束：

- 摄像头和麦克风照 ADR-0033 的逐插件授权：Android `onPermissionRequest` 转系统运行时权限（`CAMERA`、`RECORD_AUDIO`），
  iOS 用 `WKUIDelegate` 的 `requestMediaCapturePermissionFor`。只放页面自己的源；widget 页不另开一套。
- 壳不解析 widget 报文，只把字符串交给核心、把核心给的脚本注入回去，同现有的桥。
- 界面文字（入口按钮、“通话中”、中继提示、确认页的能力类别）进各平台的本地化资源，四种语言。

**iOS 的回环候选：** WKWebView 的 WebRTC 能不能用 `127.0.0.1` 的 ICE 候选没验（WebKit 历史上过滤回环候选）。不通时的退路，
按顺序试：

1. 映射另监听一个 WebKit 不当回环过滤的本机地址，LiveKit 改报这个地址（要先验 WebKit 认不认、iOS 上绑不绑得上）；
2. iOS 改用 LiveKit 的原生 Swift SDK 自画通话页，信令和授权照旧走 MatrixRTC，第 2 节的 widget 能力在 iOS 上用不上，工作量
   大一截。

## 没选的做法

- **旧的 1:1 `m.call.*`：** Element X 不支持，matrix-rust-sdk 没有信令；要自己做两端的 WebRTC 栈和状态机，还和 Element 生态分叉。
- **两端用原生 LiveKit SDK 自画通话界面：** 界面、设备切换、加密、举手、屏幕共享都要两端各做一遍，Element Call 已经有。
  只作为 iOS 回环候选不通时的退路。
- **Element Call 的 embedded 包打进 App：** 分发 AGPL 代码；每次升级 Element Call 都要发 App。
- **Jitsi：** 不是 Matrix 原生的通话，房间里看不到谁在通话，要另一套服务；Element 自己也从 Jitsi 转到了 MatrixRTC。
- **核心专为 Element Call 写宿主**（用 matrix-sdk 的 `new_virtual_element_call_widget` 写死它的参数和能力）：违背 ADR-0015，
  核心就和一个插件绑死了。改成通用的 widget 声明，Element Call 的东西都在插件清单里。
- **核心自己实现 MSC4195 授权服务**（不要 lk-jwt-service）：要自己做令牌签发和 delayed leave 代管，现成的就有。
- **网关拦 `/rtc/transports` 和 `.well-known` 自己回答：** Tuwunel 已经按标准公布，核心再改写 Matrix 接口的回答，两处会说
  不一样的话。
- **`livekit_url` 指向插件自己的地址：** 地址随插件名和服务器名变，装卸插件要改 Tuwunel 配置、重启聊天服务器。
- **MSC4108**（扫码登录、换设备）和通话无关，不在这篇。

## 代价与风险

- **lk-jwt-service 的独立模式上游已标弃用**，要转 application service 模式（MSC4502、MSC4512）。Tuwunel 支持之前只能用独立
  模式、固定版本；上游真删了时要么等 Tuwunel，要么由网关替它做 MSC4512 的代理。
- **公共中继下通话基本不可用**：约 1 Mbit/s、首帧 6 秒。只能如实提示，自建中继（ADR-0048）才缓解。
- **离开页面就挂断**，体验不如 Element X；后台通话和响铃一起补。
- **通话插件会带“高”级提示**（要撤回自己的回应），主人可能被吓到；提示文字要写清只在打开它的房间里。
- **头像可能不显示：** Element Call 用 `baseUrl` 直接取服务器上的媒体，WebView 经网关取 Matrix 媒体要访问令牌。不影响通话。
- **lk-jwt-service 连不上别名时，error 级日志里带 OpenID 令牌**（见“验证结果”）。只在核心没在跑、地址不通时出现；
  这个令牌只用来证明身份，约一小时过期；上游没有开关能关掉。接受，不为它关掉授权服务的日志，也不给上游镜像套滤日志的包装。
- 映射开着时手机上别的 App 也连得上 LiveKit 的媒体端口（ADR-0072 的风险）；LiveKit 只认带令牌的参会者。

## 待验证

- **iOS WKWebView：** `127.0.0.1` 的 ICE 候选、`requestMediaCapturePermissionFor`、信令 WebSocket 经本地代理。不通按第 5 节的退路。
- **delayed events 的上限**和 Element Call 的用法对不对得上。
- **网页请求 Matrix 对外地址（真机）：** 见“验证结果”的设备门一条；`baseUrl` 取头像能否显示。
- **Element Call 在顶层 WebView 里的 widget 模式：** 注入脚本拦下它发给自己的消息后，它的收发是否完整；`parentUrl` 设成
  页面自己的 origin 是否够。
- **加密房间：** 每人的媒体密钥经加密 to-device 交换，两台 Wonton 设备和一台 Element X 三方互通。

## 验证结果

Tuwunel v2.0.0、lk-jwt-service 0.9.0、livekit-server v1.13.11，Linux + Docker：

- `allow_federation = false` 时 `GET /_matrix/federation/v1/openid/userinfo` 回 200；`/rtc/transports` 回配置的
  `livekit_url`；没配 client 的 `.well-known/matrix/client` 回 404。回归测试钉住第一条。
- lk-jwt-service 用 reqwest + rustls-platform-verifier（Linux 上读 `SSL_CERT_FILE`/`SSL_CERT_DIR`）。别名的证书它不认、
  什么都不配时校验失败（UnknownIssuer），退到 8448 也失败，`/sfu/get` 回 401。
- `SSL_CERT_FILE` 指向“系统根证书 + 核心的 CA”的证书包时回 200（`gateway/matrix_rtc_docker_tests.rs`）。只放核心的 CA 也回
  200，但同一个授权服务就验不了别的服务器的用户；`LIVEKIT_INSECURE_SKIP_VERIFY_TLS` 也回 200，但谁的证书都认。
- 代管 delayed leave：带 `delay_id` 的 `/sfu/get` 回 200（授权服务经别名找到客户端接口）；经别名发的 `restart`、`send`
  不带访问令牌，Tuwunel 回 200，`send` 之后房间里有这条状态事件。
- 日志：授权服务连得上别名时，OpenID 令牌对、错、代管都走一遍，容器日志里没有令牌（同一个测试钉住）。连不上别名时
  （核心没在跑、地址不通）它报 `error sending request for url (…?access_token=<令牌>)`，令牌原样进日志；这是 error 级，
  `LIVEKIT_LOG_LEVEL` 只调它自己 info/debug 的多少，关不掉；不读 `RUST_LOG`。
- full access 的服务器名换到令牌后，授权服务建 LiveKit 房间成功。
- 网关测试：真实 Tuwunel 发的 OpenID 令牌经 `/_wonton/matrix-rtc/sfu/get` 换回 LiveKit 令牌，假令牌 401，没装提供方 503
  （`gateway/matrix_rtc_docker_tests.rs`）。
- 设备门：App 的本地代理按主机放行家里服务器的域名，插件页请求 `matrix.<base>/_wonton/matrix-rtc` 带着隧道身份到网关，
  过得了设备门；预检和 CORS 由授权服务自己答。这条只从代码和网关测试得出，没在真机上跑。

## 和其他 ADR 的关系

- **ADR-0014**：通话的底座定为 MatrixRTC + LiveKit 插件；Bot 作为参与方加入、屏幕共享、后台不断线留给后续。
- **ADR-0015**：核心只加通用的“插件页作为 widget”，Element Call 的参数和能力都在插件清单里。
- **ADR-0033**：桥加 widget 报文；摄像头和麦克风授权照旧；`matrix_widget` 和跨源隔离互斥。
- **ADR-0036、ADR-0057**：widget 能力按同样的“声明 + 分级提示”确认。
- **ADR-0053**：`matrix_rtc` 是一项 `provides` 能力。
- **ADR-0063**：服务器名的核心别名同它的做法。
- **ADR-0072**：媒体端口映射；它的验证结果是这篇的前提。
