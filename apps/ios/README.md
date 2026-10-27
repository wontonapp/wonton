# iOS 客户端

SwiftUI 原生壳，外加通知扩展（解密推送）。状态和业务逻辑都在 `crates/wonton-client`，本目录只负责
渲染 ViewModel、把用户操作发回核心，以及调用平台能力。
