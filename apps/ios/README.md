# iOS 客户端

SwiftUI 原生壳，外加通知扩展（解密推送）。状态和业务逻辑都在 `crates/wonton-client`，本目录只负责
渲染 ViewModel、把用户操作发回核心，以及调用平台能力。

## 结构

- `Wonton/`：App 源码。`ChatModel`、`HomeModel`、`PluginsModel` 持有核心推来的 ViewModel 并转发操作，`TimelineModel` 按顺序
  应用时间线的增量，视图只渲染。`PhotoUpload` 把相册里选的照片转成要发的图片（规则见 ADR-0005）。`FilesModel`、`FileCardView`、`TransfersStrip` 和 `BookmarkFileOpener` 是文件的选择、卡片、进度和打开（ADR-0020）。`PluginWebView`
  把 WKWebView 的 HTTP 代理设为核心的本地代理（CONNECT 加会话凭据），插件网页经隧道打开。`PairingView` 是扫码配对页，
  `QRScannerView` 用 VisionKit 的 `DataScannerViewController` 识别二维码；`InviteView` 是“添加设备”，用 Core Image
  把核心要来的配对码画成二维码。`HomeView` 配上以后是插件的主屏，连接和“添加设备”在右上角的“连接”里；`StoreView`、
  `PluginDetailView`、`InstallConfirmView` 是商店、详情和安装确认页（ADR-0017）。`SettingsView` 是底栏的设置页（ADR-0028）。`EgressSettingsView` 是“连接”里的出网代理设置，也放着插件网页局域网端口的开关（ADR-0023）。
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
增删 Swift 文件后重新运行 `xcodegen`。模拟器只支持 Apple 芯片的 Mac。

真机要用自己的开发团队签名：在 Xcode 里登录 Apple ID 后，命令行这样构建（团队 ID 是开发证书的 OU）：

```bash
xcodebuild -project apps/ios/Wonton.xcodeproj -scheme Wonton -destination "id=<设备 UDID>" -allowProvisioningUpdates DEVELOPMENT_TEAM=<团队 ID> build
```

用 `xcrun devicectl device install app --device <设备 UDID> <Wonton.app 路径>` 安装。第一次装要在手机的
“设置 → 通用 → VPN 与设备管理”里信任开发者证书；第一次打开配对页时系统请求相机权限，第一次连家里的服务器时请求
“本地网络”权限。

## 连家里的服务器（开发）

先起本地核心（`deploy/dev/core/dev.sh up`）。还没配对时“家”页面就是配对页：真机扫局域网状态页上或
`dev.sh pair` 打出的二维码；模拟器没有相机，粘贴 `dev.sh pair` 输出的最后一行，或者启动时经环境变量传入
（Debug 构建才读）：

```bash
SIMCTL_CHILD_WONTON_DEV_PAIRING_CODE="$(deploy/dev/core/dev.sh pair | tail -n 1)" xcrun simctl launch --console-pty booted app.wonton.ios
```

- 配上以后连接信息存在 App 的数据目录里，之后直接启动也会自动重连，不用再传码；配对码只能用一次。
- `WONTON_DEV_OPEN_WEB=<插件网页地址>`（比如装好 Grafana 示例插件后的 `http://app-grafana.wonton.localhost:18480/`）
  让 App 停在“家”，本地代理就绪后自动打开这个网页，并把加载结果打到控制台，真机上不用点按也能测。真机用
  `xcrun devicectl device process launch --console --environment-variables` 传同样的变量；码里的直连地址写 Mac 的
  局域网 IP（`dev.sh pair <IP>:18482`，核心要 `WONTON_DEV_TUNNEL_BIND=0.0.0.0`）。
- `WONTON_LOG` 换核心的日志过滤规则，例如 `wonton_client=info,wonton_tunnel=debug,warn` 打开本地代理的
  逐连接日志。
- `WONTON_DEV_RELOAD_ON_FOREGROUND=1` 让网页每次从后台回来都重新加载，并把加载过程带上 UTC 时间打到控制台。
  测前后台切换时，用 `xcrun devicectl device process launch --device <设备 UDID> com.apple.Preferences`
  把 App 切到后台，等若干秒再 `launch app.wonton.ios` 拉回来，对照控制台里的“回到前台，隧道检查完成”
  和“网页加载完成”。

## 聊天（开发）

“聊天”页登录主人的 Matrix 账号，家里的核心要接好 Tuwunel（`deploy/dev/core` 自带）；登录、本地存储和恢复密钥见
[ADR-0005](../../docs/adr/0005-native-clients-thick-core.md) 的“App 内聊天的核心接口”。要有个能聊的对象，装上
[回声 Bot 示例插件](../../plugins/examples/echo-bot/README.md)。

图片上传进度未知不等于 0%：已知比例时显示线性条，未知时显示转圈。

## 检查

```bash
swiftlint lint --strict --config apps/ios/.swiftlint.yml apps/ios
```

函数体超过 100 行、文件超过 800 行即报错。CI 见 `.github/workflows/ios.yml`：SwiftLint 在 Linux
上跑，macOS 上只做一次模拟器构建。

Bundle ID `app.wonton.ios` 是临时的，上架前确定正式的。
