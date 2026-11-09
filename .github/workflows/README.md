# CI 的 runner

作业全部跑在 GitHub 托管 runner 上（仓库公开后 Linux 和 macOS 分钟都免费，公开仓库不按 10 倍计 macOS）：

- `ubuntu-24.04`：lint（结构门禁、冲突标记、密钥扫描）、Rust 的格式/Clippy/测试、Android（SDK 和 NDK 27 镜像自带）、
  “Rust Docker 测试”（Docker 28 镜像自带）、核心镜像和发版镜像的 amd64 构建。
- `ubuntu-24.04-arm`：发版镜像的 arm64 构建，和 amd64 各自构建后由 merge 作业合成多架构 manifest。
- `macos-26`：SwiftLint。
- `xcode-27`：iOS 模拟器构建。选这个镜像是为对上开发机（mini）的 Xcode 27.1；作业里显式
  `xcode-select /Applications/Xcode_27.1.app`，镜像以后把默认版本升上去也不受影响。

## 什么时候跑

- 每个工作流都按 `paths` 跳过不相关的改动，范围写在各个工作流开头（lint 例外：冲突标记和密钥可能写在
  任何文件里，不按路径过滤；它里面的结构门禁在只改文档时自己跳过）。没有分支保护，跳过的检查不挡合并。
- 草稿 PR 只跑轻检查（lint 全部、Rust 的格式）；`ready_for_review` 事件触发 Clippy、测试、Docker、
  Android、iOS（各作业 `if` 里判断 `github.event.pull_request.draft`）。
- 合并进 main 不重跑全套（PR 已在最新 main 上跑过）；只有 main-image 继续构建核心镜像。每天定时
  （`schedule`，02:00 起错开）把 Rust、Docker、Android、iOS 全量跑一遍兜底。
- `test-images.yml` 每周一 02:00 UTC 把测试和镜像构建要拉的上游镜像复制到 `ghcr.io/<owner>/test-images`
  （`scripts/mirror-test-images.sh` + crane copy，digest 原样保留），也可以手动触发。托管 runner 的
  出口 IP 拉不动 Docker Hub 和 ECR Public（实测 `registry:2` 返回 toomanyrequests、构建基础镜像 429），
  所以：Docker 集成测试作业设 `WONTON_TEST_IMAGES`，测试代码经 wonton-test-images crate 把镜像引用指到
  副本，本地不设、按上游引用拉；main-image 和 release 给 Dockerfile 的 `*_IMAGE` build-arg 传副本引用。
  仓库公开当天先手动跑一次 test-images.yml，再让 main-image 跑（副本空着时镜像构建会拉取失败）。

## 合并队列

仓库开着合并队列时，入队的 PR 会在队列里的合并提交上重跑一遍必需检查；`merge_group` 事件没报告，入队就
直接判失败。所以凡是必需检查所在的五个工作流（`lint`、`rust`、`rust-docker`、`android`、`ios`）都带
`merge_group` 触发（`types: [checks_requested]`，只对 `main`）：lint 的三个作业（结构门禁、冲突标记、
密钥扫描）、rust 的三个作业（格式、Clippy、测试）、rust-docker 的 Docker 测试作业（含官方索引检查）、
android 的构建、ios 的 SwiftLint 和模拟器构建，都必须在队列里跑到。`main-image` 和 `test-images` 不用带，
队列合进 main 时该跑的照常由 `push` 触发。

`merge_group` 只支持 `branches`/`branches-ignore`，不认 `paths`，因此路径过滤只挂在 `pull_request` 下、
只在 PR 上生效；队列里不管改了什么都跑全套（`lint.yml` 本来就不按路径过滤）。作业里判断草稿的
`github.event_name != 'pull_request' || github.event.pull_request.draft == false` 在 `merge_group` 上靠前半句
成立（队列只收已转 ready 的 PR）；`github.sha` 是队列里的合并提交、`github.ref` 是
`refs/heads/gh-merge-queue/main`，所以并发组在 `merge_group` 上取 `github.event.merge_group.head_sha`，
不让几次入队挤进同一组互相取消。缓存仍只在 main 上写，队列运行和 PR 一样只读。

## 缓存

- Rust 作业用 Swatinem/rust-cache：缓存只在 main 上写（`save-if: github.ref == 'refs/heads/main'`，
  定时运行），PR 只读。三个测试分片共用 `rust-test` 一个 key（编的是同一套测试二进制），“Rust Docker
  测试”也恢复它；Clippy 和 iOS 各有各的 key（iOS 是 macOS 产物，不能和 Linux 混）。测试用 cargo-nextest
  按测试名哈希分三片并行；并发上限、卡死时限和 Docker 重活的分组在 `.config/nextest.toml`。nextest 不跑
  doctest，第一片里单列一步 `cargo test --doc`。
- 镜像构建用 buildx 的 `type=gha` 缓存。`RUN --mount=type=cache` 不会随层导出，所以 Dockerfile 用
  cargo-chef 把依赖编进独立的依赖层：清单不变就不重编依赖，gha 缓存才能省到时间。
- 仓库的 actions 缓存上限 10 GB，Rust、Gradle、buildx 的缓存都从这里出，超了按最久未用逐出。

## 安全（仓库公开后对外部 PR 也会执行）

- 每个工作流顶层 `permissions: contents: read`，需要的作业单独放开（`packages: write` 推镜像、
  `packages: read` 拉私有的测试镜像副本）。
- 所有 `actions/checkout` 都带 `persist-credentials: false`，不在工作区留 git 凭据。
- 第三方 action 固定到提交 SHA（行尾注释写版本号）；SwiftLint、XcodeGen、gitleaks、crane、actionlint
  这类下载的二进制都校验 sha256。
- 不用 `pull_request_target`；artifact 只留测试输出，保留 7 天。

## 已知边界

- `xcode-27` 是公开预览镜像；它转正或标签改名时改一下 `runs-on` 就行。
- 托管 runner 的 Docker 是经典存储（overlay2），不是 containerd 镜像存储：#666 的行为（containerd
  存储下镜像记录清掉后探针退回按创建引用起）复现不了，那个测试只在开了 containerd 存储的机器上手动跑。
- 密钥扫描用 gitleaks 官方二进制（版本和 sha256 固定在工作流里），只扫 PR 带进来的提交（`HEAD^1..HEAD`）
  或 push 到 main 的这一个提交，不扫全部历史；误报白名单在 `.gitleaks.toml`，每条写明理由，真密钥不能
  进白名单。
