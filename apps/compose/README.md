# Compose 客户端

Kotlin Multiplatform 工程：共享 UI 模块、Android 应用，以及桌面端（M2，Compose Multiplatform）。
状态和业务逻辑都在 `crates/wonton-client`，本目录只负责渲染 ViewModel、把用户操作发回核心，
以及调用平台能力。
