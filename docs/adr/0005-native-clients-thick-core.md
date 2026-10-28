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
