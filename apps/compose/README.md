# Compose 客户端

Kotlin Multiplatform 工程：共享 UI 模块、Android 应用，以及桌面端（M2，Compose Multiplatform）。
状态和业务逻辑都在 `crates/wonton-client`，本目录只负责渲染 ViewModel、把用户操作发回核心，
以及调用平台能力。

## 结构

- `shared/`：共享 UI 模块（KMP 库）。`buildRustCore` 任务用 cargo-ndk 编译 Rust 核心，并生成
  UniFFI 的 Kotlin 绑定，`.so` 和绑定都不提交。
- `androidApp/`：Android 应用，只有 `Application`（持有唯一的核心实例）和 `Activity`。
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

## 检查

```bash
./gradlew detekt
```

规则在 `config/detekt.yml`：函数超过 100 行即失败。detekt 没有文件行数规则，文件 ≤ 800 行由仓库的
`scripts/check-file-size.sh` 检查。CI 见 `.github/workflows/android.yml`。

包名 `app.wonton.*` 是临时的，上架前确定正式的。
