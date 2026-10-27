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
