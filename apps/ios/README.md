# iOS 客户端

SwiftUI 原生壳，外加通知扩展（解密推送）。状态和业务逻辑都在 `crates/wonton-client`，本目录只负责
渲染 ViewModel、把用户操作发回核心，以及调用平台能力。

## 结构

- `Wonton/`：App 源码。`SessionListModel`、`HomeModel` 持有核心推来的 ViewModel 并转发操作，视图只渲染。
  `PluginWebView` 把 WKWebView 的 HTTP 代理设为核心的本地代理（CONNECT 加会话凭据），插件网页经隧道打开。
- `WontonClient/`：Rust 核心的 Swift 包。`build/WontonClientFFI.xcframework` 和
  `Sources/WontonClient/Generated/` 由脚本生成，不提交；手写的只有把订阅包装成 `AsyncStream`
  等几行胶水代码。
- `project.yml`：XcodeGen 工程描述。`Wonton.xcodeproj` 和 `Wonton/Info.plist` 由它生成，不提交。
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

## 连家里的服务器（开发）

M0 还没有扫码配对。先起本地核心（`deploy/dev/core/dev.sh up`，装好 Grafana 示例插件），用 `dev.sh ticket`
生成连接信息，启动时经环境变量传给 App（Debug 构建才读）：

```bash
SIMCTL_CHILD_WONTON_DEV_TICKET="$(deploy/dev/core/dev.sh ticket)" SIMCTL_CHILD_WONTON_DEV_OPEN_WEB=1 xcrun simctl launch --console-pty booted app.wonton.ios
```

- 第一次启动时核心生成设备密钥，控制台打印 `本机设备身份 device_id=…`，“家”页面也显示它；用
  `dev.sh allow <设备 ID>` 登记后，插件网页直接以主人进入。
- `WONTON_DEV_OPEN_WEB=1` 让 App 停在“家”，本地代理就绪后自动打开连接信息里的网页，并把加载结果打到控制台，
  真机上不用点按也能测。真机用 `xcrun devicectl device process launch --console --environment-variables`
  传同样的变量，直连地址写 Mac 的局域网 IP（核心要 `WONTON_DEV_TUNNEL_BIND=0.0.0.0`）。
- 连接信息存在 App 的数据目录里，之后直接启动也会自动重连。

## 检查

```bash
swiftlint lint --strict --config apps/ios/.swiftlint.yml apps/ios
```

函数体超过 100 行、文件超过 800 行即报错。CI 见 `.github/workflows/ios.yml`：SwiftLint 在 Linux
上跑，macOS 上只做一次模拟器构建。

Bundle ID `app.wonton.ios` 是临时的，上架前确定正式的。
