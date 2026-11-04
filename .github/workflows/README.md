# CI 的 runner

作业跑在自托管 runner 上。

- 标签 `self-hosted, macOS, ARM64, xcode` 的机器跑 iOS 模拟器构建，要装 Xcode 和 rustup。
- 标签 `self-hosted, Linux, X64, docker` 的机器跑其余作业（结构门禁、Rust、Docker 集成测试、SwiftLint 容器、Android），要装 rustup 和 Docker，Android SDK 与 NDK 由 runner 环境的 `ANDROID_HOME` 给出。
- Rust 构建缓存放在 runner 环境 `WONTON_CI_CACHE` 指的目录（没设时是 runner 工作目录下的 `_cargo-target/`），同一台机器上的几个 runner 共用、跨次沿用。CI 关掉增量编译、调试信息只留行号，缓存小一半以上。
- 同一台 Linux 机器可以注册多个 runner，作业并行跑。
