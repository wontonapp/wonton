# CI 的 runner

作业跑在自托管 runner 上。

- 标签 `self-hosted, macOS, ARM64, xcode` 的机器跑 iOS 模拟器构建，要装 Xcode 和 rustup。
- 标签 `self-hosted, macOS, ARM64, swiftlint` 的机器跑 SwiftLint，不编译，但要装 Xcode（SwiftLint 用它的 SourceKit）；可以和 `xcode` 标在同一台上。
- 标签 `self-hosted, Linux, X64` 的机器跑结构门禁和 Android，要装 rustup，Android SDK 与 NDK 由 runner 环境的 `ANDROID_HOME` 给出。
- 带 `rust` 标签的机器跑 Rust 作业（格式、Clippy、不依赖 Docker 的测试），要装 rustup。带 `docker` 标签的主机不打 `rust` 标签，免得编译最重的作业落到最忙的机器上。只给 Rust 作业用的机器注册时加 `--no-default-labels --labels rust`，没有 `self-hosted, Linux, X64`，就不会接到 Android 这类要别的环境的作业。
- 同时带 `docker` 标签的机器还跑“Rust Docker 测试”作业（Docker 集成测试、官方索引检查），要装 Docker。
- “Rust Docker 测试”跑 Docker 集成测试前会清理测试进程已退出且超过 20 分钟的测试容器和网络，以及带 `org.testcontainers.managed-by=testcontainers` 标签、停在 `Created` 且超过 20 分钟的孤儿容器，避免被取消的作业长期占用 Docker 资源；正式安装和插件验证资源不在范围内。
- Rust 构建缓存放在 runner 环境 `WONTON_CI_CACHE` 指的目录（没设时是 runner 工作目录下的 `_cargo-target/`），按 runner 分目录、跨次沿用。Android 作业的 `GRADLE_USER_HOME` 也在这里。CI 关掉增量编译、调试信息只留行号，缓存小一半以上。Rust 的缓存在作业开始时超过 `WONTON_CI_CACHE_MAX_GB`（runner 环境，没设是 30）就整个清掉，盘紧的机器在 runner 的 `.env` 里设小。
- 同一台 Linux 机器可以注册多个 runner，作业并行跑。
- PR 上按路径跳过作业，范围写在各个工作流开头的 `paths` 里（冲突标记检查没写 `paths`，不跳过）。没有分支保护，跳过的检查不挡合并。
- 密钥扫描（secret-scan.yml）也没写 `paths`，密钥可能写在文档里。用 gitleaks 官方二进制（版本和 sha256 固定在工作流里），只扫 PR 带进来的提交（`HEAD^1..HEAD`）或 push 到 main 的这一个提交，不扫全部历史；误报白名单在 `.gitleaks.toml`，每条写明理由，真密钥不能进白名单。
