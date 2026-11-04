# Compose 客户端

Kotlin Multiplatform 工程：共享 UI 模块、Android 应用，以及桌面端（M2，Compose Multiplatform）。
状态和业务逻辑都在 `crates/wonton-client`，本目录只负责渲染 ViewModel、把用户操作发回核心，
以及调用平台能力。

## 结构

- `shared/`：共享 UI 模块（KMP 库）。`buildRustCore` 任务用 cargo-ndk 编译 Rust 核心，并生成
  UniFFI 的 Kotlin 绑定，`.so` 和绑定都不提交。
- `androidApp/`：Android 应用，只有 `Application`（持有唯一的核心实例）和 `Activity`，以及把下载好的文件交给系统打开的
  `FileProvider`（ADR-0020）。
- `scripts/build-core.sh`：被 `buildRustCore` 调用，一般不需要手动运行。

UniFFI 的 Kotlin 绑定依赖 JNA，只能编译到 JVM 类目标，不能放进 `commonMain`。所以绑定和界面暂时
放在 `shared/src/androidMain`；M2 加桌面端时，新增 JVM 目标，把它们移到 Android 与桌面共用的
源集。

## 构建

需要 JDK 21、Android SDK（platform 37 和 NDK）、Rust 的 Android target 和 cargo-ndk：

```bash
rustup target add aarch64-linux-android
```

```bash
cargo install cargo-ndk --locked
```

SDK 位置写在 `apps/compose/local.properties`（`sdk.dir=...`，不提交）或环境变量 `ANDROID_HOME`。
在 `apps/compose` 下构建 Debug APK：

```bash
./gradlew :androidApp:assembleDebug
```

默认只编译 `arm64-v8a`（真机和 Apple 芯片上的模拟器）。要别的 ABI 或 release 构建，用
`-Pwonton.abis="arm64-v8a x86_64"`、`-Pwonton.rustProfile=release`。

## 连家里的服务器（开发）

起本地核心后（`deploy/dev/core/dev.sh up`），还没配对时“应用”页就是配对页：扫局域网状态页上或 `dev.sh pair` 打出的
二维码（CameraX 取景、ZXing 识别），也可以粘贴码的文字。配上以后“应用”页右上角“连接”里的“添加设备”向核心要一个码，用 ZXing
画成二维码给新设备扫。模拟器里用 `dev.sh pair 10.0.2.2:18482` 发码（模拟器里
`10.0.2.2` 就是 Mac），经 intent 把最后一行传给 Debug 构建：

```bash
adb shell "am start -n app.wonton.android/.MainActivity --es wonton_pairing_code '$(deploy/dev/core/dev.sh pair 10.0.2.2:18482 | tail -n 1)'"
```

- 配上以后连接信息存在 App 的数据目录里，之后直接启动也会自动重连，不用再传码；配对码只能用一次。
- `--es wonton_open_web <插件网页地址>`（比如装好 Grafana 示例插件后的 `http://app-grafana.wonton.localhost:18480/`）
  让本地代理就绪后自动打开这个网页，加载结果在 logcat 的 `WontonWeb` 标签下。
- 核心日志写在 App 的 `files/core.log`（`--es wonton_log` 改过滤规则）：
  `adb shell run-as app.wonton.android cat files/core.log`。
- WebView 的代理由 androidx.webkit 的 `ProxyController` 设置，对整个 App 生效。

## 聊天（开发）

“聊天”页登录主人的 Matrix 账号，家里的核心要接好 Tuwunel（`deploy/dev/core` 自带）；登录、本地存储和恢复密钥见
[ADR-0005](../../docs/adr/0005-native-clients-thick-core.md) 的“App 内聊天的核心接口”。要有个能聊的对象，装上
[回声 Bot 示例插件](../../plugins/examples/echo-bot/README.md)。

Debug 构建默认编译 debug 的 Rust 核心，加上 matrix-sdk 后 `.so` 将近 1 GB，装进模拟器很慢，编译产物也很占磁盘；只看
界面时用 `-Pwonton.rustProfile=release`。

## 检查

```bash
./gradlew detekt
```

规则在 `config/detekt.yml`：函数超过 100 行即失败。detekt 没有文件行数规则，文件 ≤ 800 行由仓库的
`scripts/check-file-size.sh` 检查。CI 见 `.github/workflows/android.yml`。

包名 `app.wonton.*` 是临时的，上架前确定正式的。

## 推送

App 被杀或锁屏时，新消息经 UnifiedPush 分发器（如 ntfy）弹出系统通知，见 ADR-0003 的“设计：加密推送”。启动时先申请
通知权限，再向分发器登记，手机上要装一个分发器；推送的接收、解密和通知在 `androidApp` 的 `push` 包里。
