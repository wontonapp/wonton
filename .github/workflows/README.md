# CI 的 runner

作业跑在自托管 runner 上。

- 标签 `self-hosted, macOS, ARM64, xcode` 的机器跑 SwiftLint 和 iOS 模拟器构建，要装 Xcode 和 rustup。
- 标签 `self-hosted, Linux, X64, docker` 的机器跑其余作业（结构门禁、Rust、Docker 集成测试、Android），要装 rustup 和 Docker，Android SDK 与 NDK 由 runner 环境的 `ANDROID_HOME` 给出。
- Rust 构建缓存放在 runner 环境 `WONTON_CI_CACHE` 指的目录（没设时是 runner 工作目录下的 `_cargo-target/`），按 runner 分目录、跨次沿用。Android 作业的 `GRADLE_USER_HOME` 也在这里。CI 关掉增量编译、调试信息只留行号，缓存小一半以上。
- 同一台 Linux 机器可以注册多个 runner，作业并行跑。
- PR 上按路径跳过作业，范围写在各个工作流开头的 `paths` 里。没有分支保护，跳过的检查不挡合并。
