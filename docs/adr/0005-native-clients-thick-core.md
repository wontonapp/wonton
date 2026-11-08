# ADR-0005 客户端：原生三端 + 厚 Rust 核心

- 状态：已接受（2026-10-28），M0 验证

## 背景

旧客户端是三套独立实现：iOS（SwiftUI）、Android（Compose）、Desktop（Electron），每套 6 到 12 万行，
各自写聊天、数据面和设备逻辑，靠文档约束对齐，持续漂移。讨论中考虑过 Tauri 单代码库，最终决定保留
原生体验。

## 决策

- **UI**：iOS 用 SwiftUI；Android 用 Jetpack Compose；桌面用 Compose Multiplatform，与 Android
  共享 UI 模块（M2）。实际维护两套 UI。
- **厚核心**：所有状态、业务逻辑、网络（matrix-rust-sdk、iroh）、存储和 ViewModel 都在 Rust 客户端
  核心里，经 UniFFI 导出“操作进、ViewModel 出”的接口。原生壳只负责渲染 ViewModel、把用户操作发回
  核心，以及调用平台能力（推送、通知扩展、相册、文件选择、后台音频）。
- **测试**：行为测试写在核心里一次，各端共享。
- **Web**：MVP 只有服务器上的最小网页（配对二维码、运行状态）。核心接口保持可序列化；M3 把 Compose
  UI 编译到 Web（Kotlin/Wasm），核心在服务器上运行，浏览器经 WebSocket 收发操作和 ViewModel。
- **许可**：matrix-rust-sdk（Apache-2.0）、iroh（MIT/Apache-2.0）、UniFFI（MPL-2.0）可以商用；
  Element X、FluffyChat、Dart 版 Matrix SDK 是 AGPL，不使用。

聊天备份默认开启：第一台设备配对后自动建密钥备份和秘密存储，恢复密钥只给主人，聊天列表提醒保存；换手机时输入，丢了就重置，旧消息取不回。主人 2026-11-05 选择。没选的：核心保管恢复密钥、配对后自动恢复（省事，手机全丢了也能恢复，但配对码泄露、核心被攻破或硬盘被偷时多丢全部旧消息和交叉签名私钥）；只靠旧手机转发密钥（要旧手机在身边）；它后来作为恢复密钥之外并列的第二条路加上，见 [ADR-0046](0046-device-verification.md)。这样配对给的权限和安全边界都不变：配对的设备是主人，但拿不到配对前的加密消息，除非有恢复密钥，或者旧手机现场对照表情确认了它。

## 理由

- 逻辑只有一份，各端只是“画法”不同，从结构上消除漂移。
- 原生 UI 保证列表滚动、输入法、系统手势和平台能力的体验。
- 可序列化的核心接口同时服务于测试、UniFFI 绑定和将来的 Web 端。

## 代价与风险

- 仍有两套 UI 代码；推送、通知扩展、后台音频等平台能力要在两端各写一份。
- UniFFI 的异步接口和回调（ViewModel 推送）需要谨慎设计，避免旧 bridge 那种手写 C ABI 加 JSON 字符串。
- Compose 的 Web 端是画布渲染，首屏较大，中文输入法和无障碍还在完善，M3 前要验证。

## 待验证（M0）

- UniFFI 导出的 ViewModel 流在 SwiftUI 和 Compose 里的订阅方式。
- matrix-rust-sdk 在 iOS 通知扩展内存限制下是否可用；不可用时通知扩展只做解密和展示。

## 验证结果（M0，2026-10-28）

验证用的是 `crates/wonton-client` 加 `crates/wonton-protocol`：UniFFI 0.32.2（proc-macro 方式）导出
会话列表，SwiftUI 和 Compose（Android）各渲染一份，能新建、选中、删除会话。两端在模拟器上渲染出同一份
ViewModel；核心拒绝操作时，两端显示的是核心给出的同一句原因。

**结论：ViewModel 用“流”推送，不用回调。**

- 接口形态：`ClientCore.dispatch(Action)` 是同步调用，不阻塞，非法操作直接返回 `CoreError`；
  `subscribe_session_list()` 返回订阅对象，壳循环 `await next()`。第一次调用立即返回当前值，之后
  在状态变化时返回最新值（tokio `watch` 通道，来不及取走的中间状态被合并）；`cancel()` 让等待中的
  `next()` 立即返回 `None`。
- 两端的订阅方式：Swift 把订阅包成 `AsyncStream`，在 `.task` 里 `for await`，视图消失时自动取消；
  Kotlin 包成 `Flow`，用 `collectAsState` 收集。两端的包装各十几行，不含业务逻辑。
- 选流不选回调的理由：
  - 核心不持有原生对象，没有跨语言的循环引用，也不用手动解除注册。回调方式要由 Rust 持有
    Swift / Kotlin 对象，生命周期只能靠约定。
  - 核心不调用原生代码，不会因为持锁回调而重入死锁。回调在 Rust 的线程上执行，壳还要自己切回主线程。
  - 合并语义是现成的：慢的订阅者只拿到最新状态。回调方式要么逐个推中间状态，要么在原生侧再合并一次。
  - Rust 侧不用为每个订阅起任务，future 由 Swift / Kotlin 的执行器驱动；以后接 matrix-rust-sdk 的
    `Stream`，也是在 `next()` 里直接 `.next().await`。
- 取消语义（在宿主机上用生成的绑定实测）：
  - Kotlin：收集协程取消时，生成代码在 `finally` 里释放 Rust future，2 ms 内结束；被取消的
    `next()` 不会吞掉之后的状态变化。
  - Swift：UniFFI 0.32.2 的 `uniffiRustCallAsync` 不处理 Task 取消。被取消的 `next()` 会一直挂到
    下一次状态变化，并把这次变化吞掉，同一订阅的下一个 `next()` 就收不到了。所以订阅提供 `cancel()`，
    Swift 包装在 `withTaskCancellationHandler` 的 `onCancel` 里调用它；这样取消后流在 1 秒内结束。
- 其他发现：
  - 两个 crate 的 Swift 代码编进同一个 Swift 模块，共用一个 C 模块：两个 `uniffi.toml` 设同一个
    `ffi_module_name`，头文件按 crate 区分，用 `uniffi-bindgen-swift --modulemap` 生成合并的
    modulemap。这是 UniFFI 对多 crate 的预期用法。
  - UniFFI 0.32 给 Swift 错误类型生成了取自 Rust `Display` 的 `description`，但没有声明
    `CustomStringConvertible`，需要在 `WontonClient` 包里补一行声明；Kotlin 的 `toString()` 直接可用。
  - UniFFI 的 Kotlin 绑定依赖 JNA，只能编译到 JVM 类目标（Android、桌面），不能放进 `commonMain`。
    M2 的桌面端可以和 Android 共用一个 JVM 源集；M3 的 Web 端（Kotlin/Wasm）用不了这套绑定，
    要从 `wonton-protocol` 另外生成可序列化的类型，到时再定。
- 后续：整份快照适合会话列表这种小 ViewModel。聊天时间线要推增量（matrix-rust-sdk 的
  `VectorDiff`），届时沿用同样的订阅形态，但推的是不能合并的增量批次。

## 验证结果（App 内聊天的核心接口，2026-10-28）

`crates/wonton-client` 的 `chat` 模块把 `wonton-matrix` 接进客户端核心，接口沿用上面的结论，并补上了时间线的
增量形态。

- **聊天状态**：`subscribe_chat()` 和会话列表、家里的连接一样推整份快照（账号、同步、加密、房间列表、后台操作
  的失败说明），中间状态被合并。
- **时间线**：`subscribe_timeline(room_id)` 返回的订阅推的是不能合并的增量批次：第一批是 `Reset`，之后 SDK 的
  `VectorDiff` 按顺序转发；成员资料晚到时另补同下标的 `Set`（见 ADR-0003）。壳按顺序应用到自己的列表上，不因补名字
  重置列表。核心内部用无界队列排队，订阅取消或释放后转发自动停止。取消语义和快照订阅一样（Swift 侧要调 `cancel()`）。
- **导出的 async 方法**（`subscribe_timeline`、`load_media`）在核心自己的 tokio 运行时里执行，再把结果交回壳：
  UniFFI 的 future 由 Swift / Kotlin 的执行器驱动，线程上没有 tokio 运行时，matrix-sdk 的网络操作离不开它。
- **登录**：跟着家里的连接走。本地代理就绪后，有这台服务器存下的登录状态就恢复会话，否则经隧道向核心要一次性登录令牌
  （ADR-0009 的“接口：App 登录和使用 Matrix”），登录 `matrix.<基础域名>`。matrix-sdk 的 HTTP 请求都走本地代理，
  带本次启动的会话凭据。核心地址取自连接信息的 `core_url`；没给时用基础域名，协议和端口跟插件网页一样。要令牌的
  请求最多等 30 秒（和 matrix-sdk 默认的请求超时一样）：隧道或本地代理收下请求却一直不回时，按“传输不可用”失败，
  不会一直停在“正在登录”。
- **暂时的登录失败自己重试**：登录因为“传输不可用”或“未就绪”（Matrix 还在启动）失败时，账号显示“正在重连”
  （`ChatAccount::Reconnecting`），核心隔 1、2、4……秒（最多 60 秒）再登录，回到前台或隧道重连时马上再登录。家里的
  连接一直连着、只是那一次请求失败（满载时网关回 502、要令牌超时）时，连接没有任何变化可等，不自己重试的话聊天就
  一直断着。被拒、输入非法、服务器出错要用户处理，停在“失败”，只在回到前台时再试一次。
- **登录状态和本地存储一起用**：登录状态（`matrix/session.json`）只在 matrix-sdk 的 SQLite 存储（`matrix/store/`）
  还在时用来恢复。存储没了还恢复，SDK 会给原来的设备 ID 配一套新的设备密钥，别的设备看到的是同一台设备换了
  密钥；所以存储不在时删掉登录状态，重新要令牌登录成一台新设备。
- **登录状态只对登录时那台服务器有效**：登录状态记着服务器隧道的 endpoint ID，和当前连接信息里的对不上时，连同本地
  存储一起丢掉，重新登录。homeserver 地址和用户认不出服务器（每台默认都是 `matrix.wonton.localhost` 上的 `@owner`），
  拿旧令牌去请求只会被拒，看着像设备被吊销。服务器换 IP、换中继时 endpoint ID 不变，会话保留。还没确认保存的恢复密钥
  不丢：它属于旧服务器，主人回到那台服务器还要用。
- **恢复密钥只显示一次**：设置恢复成功后，恢复密钥放进 `EncryptionStatus.recovery_key`，同时先存在本机（App 在
  主人抄下之前被关掉，下次启动还能看到）；主人确认已保存（`ConfirmRecoveryKeySaved`）后，核心把它从快照和本机
  一起清掉。用恢复密钥恢复时，主人输入的密钥用完即弃，不存。密钥在 Rust 里是 `SecretString`，`Debug` 不打印原文。
- **缩略图由发送端生成**：加密房间里服务器看不到图片，做不了缩略图。壳发图时附一张缩略图（iOS 和 Android 都是
  长边不超过 800 像素的 JPEG，原图长边不超过 800 像素时不附），核心和原图一起加密上传；时间线里的图片给出原图和
  缩略图两个句柄，列表里显示缩略图，点开再取原图。
- **发送经发送队列，出错后接着发**：文字和图片都经 matrix-sdk 的发送队列，先以本地回显出现，图片的原图和缩略图
  先存进本机的媒体缓存，句柄指向缓存。队列出了任何错都会停下，要嵌入方重新打开，SDK 自己不会（matrix-sdk 0.19.1
  实测，#86）：会话在后台重新打开它，连接出错的隔一段时间自动重发，服务器拒收的停在队首、挡住这个房间之后的消息，
  等主人重发或取消；会话建好时接着发上次没发完的。壳看到的行为写在 `SendState` 的文档里。验证：
  `cargo test -p wonton-matrix --test tuwunel -- --include-ignored send_queue`，客户端经一个能断开、能拒收的 TCP
  中转连 Tuwunel。
- **超过上传上限的图片当场拒绝**：发图前先取服务器的上传上限（`m.upload.size`；Tuwunel 由 `max_request_size`
  决定，v1.9.4 默认 24 MiB），图片比它大就按“输入非法”失败，说明里写出图片大小和上限，不进发送队列：进了队列，
  上传被服务器拒收后会停在队首，挡住之后的消息。SDK 只在内存里记着上限，每次启动后第一次发图才要；一时要不到
  （比如隧道断开）就不查，照样进队列，连上后由服务器判断。验证：`cargo test -p wonton-matrix --test tuwunel
  -- --include-ignored`，上限设成 10 MiB，发 11 MiB 的图。
- **回到前台时的重试**只在登录失败（不是被吊销）、正在等着重连或同步停了时做；正在登录时不插手。Android 启动时也会发一次
  “回到前台”，正赶上第一次登录，之前会再登录一遍，在同一个存储上开出两个会话：模拟器上实测请求挂住 30 秒，加密
  的恢复状态一直是“未知”。
- **验证**：`cargo test -p wonton-client --test it chat:: -- --include-ignored`（Docker 集成测试）。一次性的
  Tuwunel 前面接一个模拟的核心（签发登录令牌，把 `matrix.<基础域名>` 转给 Tuwunel）和真实的隧道服务端，手机是
  真实的客户端核心，只经导出给壳的接口驱动：
  - 连上后自动登录主人的账号；
  - 收到 Bot 的加密私聊邀请，列表里显示 Bot 的名字，接受后时间线按增量收到文字和图片，图片的原图和缩略图都取到
    解密后的原样；
  - 发出的文字（去掉首尾空白）和带缩略图的图片对方都收到，标为已读后未读清零，设置恢复后得到恢复密钥；
  - 重启后还是同一台设备，不再要登录令牌，加密的历史仍能解密，没确认保存的恢复密钥还在；确认后清掉，再重启也
    不再出现；
  - 删掉本地存储后重启，重新要令牌，登录成新设备；新设备解不开以前的消息，但发送者照样显示名字。

  整个流程几秒钟。

## 验证结果（App 内聊天的界面，2026-10-28）

`apps/ios` 和 `apps/compose` 的“聊天”页取代了 M0 演示用的“会话”页，和“家”（插件网页入口）并列，只渲染核心给的聊天
ViewModel 和时间线增量。

- 时间线沿用上面的订阅形态：壳把每一批增量按顺序应用到自己的列表（Swift 的 `@Observable` 数组、Compose 的
  `SnapshotStateList`），列表按条目 `id` 刷新。
- 发图时读相册、缩小、转码和生成缩略图都在壳里（iOS 用 ImageIO，Android 用 `ImageDecoder` 和 `Bitmap.compress`），
  缩略图的规则见上一节。
  - **尺寸**：原图长边超过 4096 像素时按 1/2、1/4… 缩，缩到不超过 4096 为止，缩完的长边大于 2048：4800 万像素的
    8064×6048 缩成 4032×3024，2400 万像素的 5712×4284 缩成 2856×2142。只按这些比例缩，是因为解码器只在这些比例上
    边解边缩：缩到别的尺寸时，Android 的 `ImageDecoder` 和 iOS 的 ImageIO 解 JPEG 都先解出不小于目标的最近一档，
    长边 4096 到 8192 的照片就要先解出原尺寸，将近 200 MB，低端 Android 机有 OOM 的风险。iOS 解 HEIC 不受这个限制，
    为了两端一致也按同样的规则缩。
  - **格式和元数据**：PNG 重新编码成 PNG（无损）；其余（JPEG、HEIC 等）转成质量 0.9 的 JPEG，因为不是每个 Matrix
    客户端都能显示 HEIC。重新编码只写像素，原图的 EXIF、GPS、XMP、IPTC 都不带出去，位置信息不外发；ImageIO 会自己
    写一段 EXIF，只有色彩空间和像素尺寸。方向按 EXIF 转正后写进像素，不靠接收方读 EXIF（Android 的 `BitmapFactory`
    就不读）。
  - **GIF 是例外**：原样发，保留动画，不缩小；文件里带的元数据（GIF 可以在扩展块里带 XMP）也原样发出去。
  - 缩完、转完仍比服务器的上传上限大的图（例如细节很多的 4096 像素 PNG）由核心拒绝，见上一节。
- **实测**：对开发核心（#53 的提交 cee0e22，带 Tuwunel 和回声 Bot 示例插件），iPhone 17 Pro 模拟器（iOS 26.5）和
  Android 模拟器（arm64，API 36），两台都是主人的设备：
  - 经隧道登录主人的账号，列表里有和“回声”的加密私聊；发文字收到 `回声：<原文>`。
  - iOS 发的照片（HEIC 转 JPEG，附缩略图）两端都显示，点开是原图。
  - iOS 设置恢复：恢复密钥在 App 重启后仍在，点“已保存”后从界面和本机都清掉。Android 这台新设备一开始是“未验证、
    缺少密钥”，以前的消息解不开；输入恢复密钥后变成已验证，以前的文字和图片都解开。
  - 两台设备各自发的消息，另一台和 Bot 都收到。
  - Android 模拟器上，经隧道的 Matrix 请求偶尔挂住，30 秒后超时（`keys/claim`、密钥备份上传、同步）；同样的核心在
    iOS 模拟器上没有，记在 #17。
- **真机**（2026-10-30）：iPhone 16（iOS 27.2），装的是这些页面加上扫码配对页的构建（提交 ae0c0fd）；服务器是局域网里
  一台 amd64 主机（Ubuntu 24.10），用公开的一行命令安装，核心镜像由 main 的 ea888d5 构建。扫码配上后（ADR-0009），手机在
  5G 下经中继连回家里，和 Hermes（模型经 magpie 走 DeepSeek）对话，发文字收到回复。发图、恢复密钥、前后台切换没在真机
  上测，Android 真机没测。
- **核对缩小和元数据**（2026-10-30）：样图是带 GPS（4 个标签）、EXIF 注释、XMP 和 IPTC 说明的 PNG（3000×2000、
  6000×4000）和 JPEG（8000×6000）。
  - iOS：`PhotoUpload.swift` 原样编译，在 macOS 27.2 上跑（ImageIO 和 iOS 是同一套框架，没在 iOS 上跑）。输出的 PNG
    只剩 IHDR、sRGB、eXIf、IDAT、IEND，JPEG 只剩 JFIF、EXIF 和一段空的 IPTC；EXIF 里只有色彩空间和像素尺寸，原图的
    GPS、注释、XMP、IPTC 都没有。GIF 输出和输入逐字节相同。
  - Android：API 36 的 arm64 模拟器，调用和 `PhotoUpload.kt` 相同的 `ImageDecoder` 和 `Bitmap.compress`。输出的 PNG
    只剩 IHDR、sRGB、sBIT、IDAT、IEND，JPEG 只剩 JFIF 和 ICC 色彩配置，没有 EXIF、XMP、IPTC。
  - 两端都把 6000×4000 的 PNG 缩成 3000×2000、8000×6000 的 JPEG 缩成 4000×3000，3000×2000 的不变。
  - 内存（8064×6048 的噪点图）：Android 解 JPEG 按 1/2 采样多占 48 MB，直接给 4096 的目标尺寸多占 232 MB；iOS 的
    ImageIO 缩到 4032 时进程峰值 211 MB，缩到 4096 时 492 MB。HEIC 在 Android 上总要先解出原尺寸，按 1/2 采样也多占
    242 MB（直接给目标尺寸 363 MB）；iOS 上缩到哪一档都在 60 MB 左右。
