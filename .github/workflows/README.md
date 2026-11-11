# CI 的 runner

作业全部跑在 GitHub 托管 runner 上（仓库公开后 Linux 和 macOS 分钟都免费，公开仓库不按 10 倍计 macOS）：

- `ubuntu-24.04`：lint（冲突标记、密钥扫描、结构门禁）、各工作流的汇总作业、Rust 的格式/Clippy/测试、Android（SDK 和 NDK 27 镜像自带）、
  “Rust Docker 测试”（Docker 28 镜像自带）、核心镜像和发版镜像的 amd64 构建。
- `ubuntu-24.04-arm`：发版镜像的 arm64 构建，和 amd64 各自构建后由 merge 作业合成多架构 manifest。
- `macos-26`：SwiftLint。
- `xcode-27`：iOS 模拟器构建。选这个镜像是为对上开发机（mini）的 Xcode 27.1；作业里显式
  `xcode-select /Applications/Xcode_27.1.app`，镜像以后把默认版本升上去也不受影响。

## 必需检查与汇总作业

- 规则集只配每个工作流末尾的汇总作业（`rust 汇总`、`rust-docker 汇总`、`ios 汇总`、`android 汇总`、
  `lint 汇总`）：`needs` 本工作流的全部作业、`if: always()`，任一依赖 failure 或 cancelled 就红，
  success 和 skipped 算过。汇总只干这一件事（一小段 shell，不引第三方 action），草稿 PR 上也起来报一个
  通过；这样才能把其余作业直接在作业级跳过，不用为了报检查先起 runner。
- 汇总的名字在 `pull_request` 和 `merge_group` 下是同一个，规则集两种事件共用一份。
- Rust 测试分片被作业级 `if` 跳过时 matrix 不展开，检查名会留成带 `${{ matrix.shard }}` 的字面量；
  规则集只看汇总，不受影响。所以分片的草稿和路径判断也在作业上。

## 什么时候跑

- 路径不在触发器上过滤：合并队列的必需检查每次入队都要有结果，工作流不触发的话检查一直“等待中”，队列
  直接判失败。改动范围改由各工作流开头的 `changes` 作业判断（`dorny/paths-filter`，只在非草稿 PR 上跑），
  没改到的重作业挂在它上面、自己报 skipped，范围还是写在各个工作流开头；跳过的检查算通过，没有分支保护，
  不挡合并。lint 例外：冲突标记和密钥可能写在任何文件里，不按路径过滤；它里面的结构门禁只查源码文件，
  只改文档的提交和草稿 PR 上跳过。
- 过滤范围按“这个工作流真会读什么”列：Rust 只列测试和预热读的部署文件（`deploy/install/`、
  `deploy/dev/{acp-bridge,matrix}/`、`deploy/dev/core/compose.yaml`、`deploy/dev/core/Dockerfile`），
  `deploy/dev` 其余的只给本机开发栈（开发索引 `core/store/`、`experiments/`、smoke 脚本），改了不跑重作业；
  新的 `include_str!` 指向 `deploy/` 时要补进列表。只改 `.md` 的 PR 每个工作流都跳过（`apps/compose/` 下的也算）。
  漏列不会漏进 main：队列不按路径判断，入队那一轮一定全套跑。
- 草稿 PR 上 `rust`、`rust-docker`、`ios`、`android` 一个作业都不起（`changes` 也算在内），只有各自的汇总
  起来报通过；lint 只跑冲突标记和密钥扫描（同一个作业的相邻两步）。转 ready（`ready_for_review`）跑全套。
- 所有 PR 运行都 `cancel-in-progress`：反正要重跑，旧的别继续占名额；定时运行跑完。
- 合并进 main 不重跑全套（PR 已在最新 main 上跑过）；只有 main-image 继续构建核心镜像，`cache.yml` 用
  和 PR 作业一样的 key 把编译缓存编一遍存下来（只编不跑）。每天定时（`schedule`，02:00 起错开）把
  Rust、Docker、Android、iOS 全量跑一遍兜底。
- 打标签（`labeled`）不进 `ios.yml`、`android.yml`：那会用一次标签事件重跑整轮日常作业，而且跳过的那轮会把之前
  失败的必需检查盖成“通过”。重录统一走 `snapshots-record.yml`，只在标签是 `record-snapshots` 时跑一个作业，
  不算必需检查。
- `test-images.yml` 每周一 02:00 UTC 把测试和镜像构建要拉的上游镜像复制到 `ghcr.io/<owner>/test-images`
  （`scripts/mirror-test-images.sh` + crane copy，digest 原样保留），也可以手动触发。托管 runner 的
  出口 IP 拉不动 Docker Hub 和 ECR Public（实测 `registry:2` 返回 toomanyrequests、构建基础镜像 429），
  所以：Docker 集成测试作业设 `WONTON_TEST_IMAGES`，测试代码经 wonton-test-images crate 把镜像引用指到
  副本，本地不设、按上游引用拉；main-image 和 release 给 Dockerfile 的 `*_IMAGE` build-arg 传副本引用。
  仓库公开当天先手动跑一次 test-images.yml，再让 main-image 跑（副本空着时镜像构建会拉取失败）。

## 合并队列

仓库开着合并队列时，入队的 PR 会在队列里的合并提交上重跑一遍必需检查；`merge_group` 事件没报告，入队就
直接判失败。所以凡是必需检查所在的五个工作流（`lint`、`rust`、`rust-docker`、`android`、`ios`）都带
`merge_group` 触发（`types: [checks_requested]`，只对 `main`）：各工作流的汇总作业要在队列里报告，
它们 `needs` 了本工作流的全部作业，队列里又只是放行不跳过，所以全套都得跑到。`main-image` 和 `test-images`
不用带，队列合进 main 时该跑的照常由 `push` 触发。

`changes` 作业只在非草稿 `pull_request` 上跑，`merge_group`、`schedule`、`workflow_dispatch` 上重作业靠
`if` 里的 `github.event_name != 'pull_request'` 直接放行（重作业的 `if` 都以 `!cancelled()` 开头：
`changes` 被跳过时下游作业才不会被连坐跳过；汇总作业用 `always()`，`changes` 跳过也照样报告）。
作业里判断草稿的 `github.event.pull_request.draft == false` 也只在 PR 上求值（队列只收已转 ready 的 PR）；
`github.sha` 是队列里的合并提交、`github.ref` 是 `refs/heads/gh-merge-queue/main`，所以并发组在
`merge_group` 上取 `github.event.merge_group.head_sha`，不让几次入队挤进同一组互相取消。缓存仍只在 main 上写，
队列运行和 PR 一样只读。

## iOS 单测和截图测试

`ios.yml` 的“模拟器构建”作业用 `build-for-testing` 加 `test-without-building`，在一台固定型号和系统版本的
模拟器（工作流开头的 `SIMULATOR_NAME`、`SIMULATOR_OS`）上只跑 `WontonTests`，包括 `SnapshotTests`
（pointfreeco/swift-snapshot-testing，MIT，钉版本，只进测试目标不链接 App）。UI 测试在夜跑里跑，这个作业
不编。参考图在 `apps/ios/WontonTests/__Snapshots__/`，直接进 git，不用 LFS。

- 参考图绑定那台模拟器和系统版本，外观、语言（en、US）和时区（UTC）钉在 scheme 和测试里。换了模拟器或系统
  要整体重录，只换图不换参数会一直红。
- 每个用例默认 10 秒、最多 60 秒，失败不自动重试。
- 录制模式靠 scheme 的环境变量切：`Wonton` 是 `SNAPSHOT_TESTING_RECORD=never`（只比较，缺一张参考图也直接红，
  不在 CI 里悄悄补图），`WontonSnapshots` 是 `all`。xcodebuild 步骤的 `env` 传不进模拟器里跑的 App，只有 scheme
  的环境变量进得去。
- 比较失败时“失败时存测试结果包”一步把 `.xcresult` 存成产物，保留 7 天；新图和差异图是里面的附注。
- 重录在 `snapshots-record.yml`：在 PR 上打 `record-snapshots` 标签（草稿 PR 的事件不跑日常作业，但重录作业
  照跑）。作业用 `WontonSnapshots` scheme 把参考图全部重画，提交回分支并摘掉标签。录制那一轮本身是红的
  （每录一张报一条“Automatically recorded snapshot”），以提交回去的图为准；它不算必需检查。
- GITHUB_TOKEN 推的提交不会触发工作流（GitHub 的规则），仓库也没有单独的机器人令牌，所以录完要自己推一个
  空提交或点重跑，新图才会被比较一遍。外部 fork 的 PR 令牌推不回对方的分支：图存成 `ios-snapshots` 产物，作者自己取。
- 新增截图用 `WontonTests/SnapshotFixtures`（Debug 专用）搭界面，只放有代表性的界面，不逐页全录。

## 缓存

- 缓存只在 main 上写，PR、合并队列和定时运行只读：仓库公开，外部 PR 写的条目会把 main 的挤掉。
  main 上不跑测试（只有 push），所以另开 `cache.yml`：push 到 main 或手动触发时，用和 PR 作业**完全相同**
  的 rust-cache key 前缀、缓存目录和编译目标只编不跑（测试二进制、Clippy、iOS 模拟器核心、Android 核心、
  桥镜像）。改这几个工作流的 rust-cache 参数（`shared-key`、`workspaces`、环境变量）时要和 `cache.yml`
  一起改，少一处 PR 就恢复不到。手动触发也写缓存：换分支验证能不能恢复时用它。
- `save-if` 要写成 `${{ … }}`：不包的时候 runner 把表达式原样当字符串交给动作，动作只认字面量 `true`，
  结果一次也存不下来（之前就是没包，调度器日志里能看到传给动作的是 `github.ref == 'refs/heads/main'` 本身）。
- Rust 作业用 Swatinem/rust-cache，三个测试分片共用 `rust-test` 一个 key（编的是同一套测试二进制），
  “Rust Docker 测试”也恢复它；Clippy、iOS、Android 各有各的 key（iOS 是 macOS 产物，不能和 Linux 混）。
  测试用 cargo-nextest 按测试名哈希分三片并行；并发上限、卡死时限和 Docker 重活的分组在
  `.config/nextest.toml`。nextest 不跑 doctest，第一片里单列一步 `cargo test --doc`，`cache.yml` 也先把
  doctest 编好。
- 镜像构建的层缓存走 registry（`type=registry`，包是 `ghcr.io/<owner>/wonton-ci-cache`），不占 10 GB 的
  actions 缓存：main-image 每次合并写回，发版只读；桥镜像的预构建从 main 的缓存恢复、只在 main 写回，
  构建完 `--load` 回本机给测试用。Dockerfile 用 cargo-chef 把依赖编进独立的依赖层：清单不变就不重编依赖，
  缓存要 `mode=max` 才存得下这一层。
- actions 缓存上限 10 GB，只剩 rust-cache 在用，超了按最久未用逐出。

## 安全（仓库公开后对外部 PR 也会执行）

- 每个工作流顶层 `permissions: contents: read`，需要的作业单独放开（`packages: write` 推镜像、
  `packages: read` 拉私有的测试镜像副本）。在 main 上写层缓存用 `packages: write`，PR 上是 `packages: read`
  （外部 PR 的令牌到不了写）。
- 所有 `actions/checkout` 都带 `persist-credentials: false`，不在工作区留 git 凭据。
- 第三方 action 固定到提交 SHA（行尾注释写版本号）；SwiftLint、XcodeGen、gitleaks、crane、actionlint
  这类下载的二进制都校验 sha256。
- 不用 `pull_request_target`；artifact 只留测试输出，保留 7 天（夜跑的截图、录屏为查失败留 14 天）。

## 界面测试夜跑（`nightly-ui.yml`）

两端的界面测试每晚在模拟器上各跑一遍，只定时和手动触发，不挂 PR、不进合并队列、不设必需检查。时间沿用
`ios.yml` 和 `android.yml` 的定时（一个工作流两条 cron，作业按 `github.event.schedule` 分开），不另占窗口。

- iOS 只开一台模拟器跑全部 `WontonUITests`；要两台的用例（粘贴的对方视角、好友两端）在工作流里排除或
  `XCTSkip`。要后端的用例没有配对码时 `XCTSkip`，不静默通过。手动触发可以填 `pairing_code`，把开发栈的
  配对码带进来；`PAIR_CODES` 也一并给。
- 不需要后端的两个用例是冷启动：全新安装落在配对页，和 `WONTON_UI_FAKE` 的假数据首屏。假数据入口整个
  在 cargo feature `ui-fake` 后面（`crates/wonton-client`）：Debug 构建默认带上，Release 不带；夜跑在
  构建时显式传开关（iOS 的 `WONTON_UI_FAKE=1`、Android 的 `-Pwonton.uiFake=1`），壳的调用在 `#if DEBUG`
  里，生产 Release 包编不进去。
- Android 在 `ubuntu-24.04` 上开 KVM，用 `ReactiveCircus/android-emulator-runner`（Apache-2.0）起一台
  x86_64 模拟器跑 `connectedDebugAndroidTest`；首屏用例不依赖服务端。
- 用例的时限由 xcodebuild 的 `-test-timeouts-enabled YES` 和 Gradle 自己的超时兜底，不加测试重试。
- 失败自动开 issue（标题 `夜跑失败：<端> <测试>`）；同一条测试已经有开着的同名 issue 就在上面留言，不重复开。
  解析不到失败用例（构建或前置步骤挂了）时报一条作业级 issue。截图、录屏和测试报告（iOS 的 `.xcresult`、
  `simctl io recordVideo`；Android 的 `screenrecord`、测试报告）存成产物保留 14 天。
- 连续两周稳定、这期间的失败都查清是真问题之后，再把“启动到首屏 + 一个关键流程”挪进 PR 的必需检查。
  挪的时候放进已有作业的步骤里、作业名不改，规则集不用动。

## 已知边界

- `xcode-27` 是公开预览镜像；它转正或标签改名时改一下 `runs-on` 就行。
- 托管 runner 的 Docker 是经典存储（overlay2），不是 containerd 镜像存储：#666 的行为（containerd
  存储下镜像记录清掉后探针退回按创建引用起）复现不了，那个测试只在开了 containerd 存储的机器上手动跑。
- 密钥扫描用 gitleaks 官方二进制（版本和 sha256 固定在工作流里），只扫 PR 带进来的提交（`HEAD^1..HEAD`）
  或 push 到 main 的这一个提交，不扫全部历史；误报白名单在 `.gitleaks.toml`，每条写明理由，真密钥不能
  进白名单。
