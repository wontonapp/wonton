# iOS 客户端

SwiftUI 原生壳，外加通知扩展（解密推送）。状态和业务逻辑都在 `crates/wonton-client`，本目录只负责
渲染 ViewModel、把用户操作发回核心，以及调用平台能力。

## 结构

- `Wonton/`：App 源码。`SessionListModel` 持有核心推来的 ViewModel 并转发操作，视图只渲染。
- `WontonClient/`：Rust 核心的 Swift 包。`build/WontonClientFFI.xcframework` 和
  `Sources/WontonClient/Generated/` 由脚本生成，不提交；手写的只有把订阅包装成 `AsyncStream`
  等几行胶水代码。
- `project.yml`：XcodeGen 工程描述。`Wonton.xcodeproj` 由它生成，不提交。
- `scripts/build-core.sh`：编译 Rust 核心、生成 XCFramework 和 Swift 绑定。

## 构建

需要 Xcode、[XcodeGen](https://github.com/yonaskolb/XcodeGen)、SwiftLint，以及 Rust 的 iOS target：

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

在仓库根目录依次运行：

```bash
apps/ios/scripts/build-core.sh
```

```bash
xcodegen generate --spec apps/ios/project.yml
```

然后用 Xcode 打开 `apps/ios/Wonton.xcodeproj` 运行。改了 Rust 代码后重新运行 `build-core.sh`；
增删 Swift 文件后重新运行 `xcodegen`。只支持 Apple 芯片的 Mac 上的模拟器。

## 检查

```bash
swiftlint lint --strict --config apps/ios/.swiftlint.yml apps/ios
```

函数体超过 100 行、文件超过 800 行即报错。CI 见 `.github/workflows/ios.yml`：SwiftLint 在 Linux
上跑，macOS 上只做一次模拟器构建。

Bundle ID `app.wonton.ios` 是临时的，上架前确定正式的。
