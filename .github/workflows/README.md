# CI 的 runner

作业全部跑在 GitHub 托管 runner 上（仓库公开后 Linux 和 macOS 分钟都免费，公开仓库不按 10 倍计 macOS）：

- `ubuntu-24.04`：lint（冲突标记、密钥扫描、结构门禁）、gate 的改动范围和汇总作业、Rust 的格式/Clippy/测试、Android（SDK 和 NDK 27 镜像自带）、
  “Rust Docker 测试”（Docker 28 镜像自带）、核心镜像和发版镜像的 amd64 构建。
- `ubuntu-24.04-arm`：发版镜像的 arm64 构建，和 amd64 各自构建后由 merge 作业合成多架构 manifest。
- `macos-26`：SwiftLint。
- `xcode-27`：iOS 模拟器构建和 Release 体积归档。选这个镜像是为对上开发机（mini）的 Xcode 27.1；作业里显式
  `xcode-select /Applications/Xcode_27.1.app`，镜像以后把默认版本升上去也不受影响。

## 合并门槛和合后测试

PR 合并前等的是快检查：格式、Clippy、detekt、SwiftLint、行数门禁、编译、不依赖 Docker 的单元测试和截图比对
（`gate.yml` 调用的 `lint`、`rust`、`ios`、`android`），目标是十分钟内出结果；另加只在改到相关路径时才跑的 Docker 集成测试
（见下面的 `rust-docker.yml`，中位二十几分钟），跑完才合。其它慢的挪到合进 `main` 之后跑，不挡合并：

- `rust-docker.yml`：依赖 Docker 的集成测试。改了它路径范围（核心的聊天、存储、推送等代码）的 PR 由 `gate.yml` 的 `docker` 作业
  调用、算合并门槛，`gate 汇总` 等它的结果，草稿 PR 不跑；`push` 到 `main` 时仍按同一份路径触发，每次合并单独跑一遍、
  不互相取消，红了能按那次提交标题里的 `(#N)` 找到弄红的 PR。`chat::push_method::background_messages_are_collected_and_cleared_on_foreground`
  已知会在同步慢时 60 秒超时（PR #1081 首次跑实测），红了先手动重跑；根因另开卡修，这里不加 retry。
- `ios.yml` 的模拟器构建（WontonTests）在只改了 wonton-client 依赖的其余 crate、依赖版本或工具链时：这些不改导出给
  Swift 的接口，很少弄坏 iOS。改到 Swift 这边或接口（`apps/ios`、设计生成、`wonton-protocol`、`wonton-client`）的
  PR 上照常跑、算必需检查：iOS 在 PR 上红的多半是这几处，挪到合后会让 main 一天红好几次。范围和理由写在 `ios.yml` 开头。
  合进 main 的连续运行只留最新那次（`cancel-in-progress`）：结果只对旧提交有意义，iOS 的作业一次要编十几分钟，
  留着旧的在争名额；被取消的旧提交按提交标题里的 `(#N)` 归到它的 PR。
- `ios-size.yml`：Release App 体积门禁。要编一份 release 核心、归档一份 App（冷缓存十几分钟），只按 `push` 到 main 的
  iOS 路径、每天定时和手动触发；失败留 dSYM 和体积报告，供还原崩溃栈。
- `main-image.yml`：核心镜像构建。
- `nightly-ui.yml`：两端的界面测试（XCUITest、设备上的 androidTest），只定时和手动。
- `release.yml`：版本标签发布核心及组件镜像；手动触发仅接受 main，按 `component` 单独发布远端 MCP 连接器，
  只写 `main-<短提交号>`，不更新核心版本或 stable。只发连接器用 `gh workflow run release.yml --ref main -f component=remote-mcp`；
  组件发布仍须先取得对应范围的许可。

合后的测试红了，谁合的谁 30 分钟内修，修不好 revert（见 AGENTS.md“Git 与协作”）。快慢的分界按“能不能十分钟内
出结果”定：新加的测试要起容器、走界面流程（UI 测试）或要构建镜像的，放进合后的工作流，不放进必需检查；
Docker 集成测试例外：新加的 Docker 测试进 `rust-docker` 套件，改到相关路径的 PR 合并前跑，UI 测试和镜像构建仍放合后。

## 必需检查：gate 汇总

- 规则集只配一个检查：`gate.yml` 末尾的 `gate 汇总`。`gate` 里一个 `changes` 作业按改动范围给出 `rust`、`ios`、
  `android`、`docker` 四个开关，再用 `workflow_call` 调用 `lint.yml` 和改到的那几个重工作流（`docker` 调 `rust-docker.yml`）；汇总 `needs` 全部、`if: always()`，
  任一依赖 failure 或 cancelled 就红，success 和 skipped 算过。汇总只干这一件事（一小段 shell，不引第三方 action），
  草稿 PR 上也起来报结果，重作业全被跳过时判红。
- 一次推送只起 `changes`、`lint`、汇总三个轻作业，加改到的重作业。之前四个工作流各有自己的改动范围和汇总，一次推送
  八个轻作业，各占一台 runner。
- 被调用的作业检查名是“调用方 / 作业名”（如 `rust / 格式`）。Rust 测试分片被跳过时 matrix 不展开，检查名会留成带
  `${{ matrix.shard }}` 的字面量；规则集只看汇总，不受影响。
- 新的快检查放进 `gate` 调用的工作流（作业或步骤都行），规则集不用动；改动范围改 `gate.yml` 的 `changes`。

## 什么时候跑

- `gate` 不在触发器上按路径过滤：必需检查每个 PR 都要有结果，工作流不触发的话检查一直“等待中”，
  PR 合不了。改动范围由 `gate.yml` 的 `changes` 作业判断（`dorny/paths-filter`，在非草稿 PR 上通过 REST
  列文件，不检出代码），没改到的重作业自己报 skipped；汇总作业把成功和跳过都算通过。
  lint 不看开关：冲突标记和密钥可能写在任何文件里，不按路径过滤；它里面的结构门禁只查源码文件，
  只改文档的提交和草稿 PR 上跳过。
- 过滤范围按“这个工作流真会读什么”列：Rust 只列测试和预热读的部署文件（`deploy/install/`、
  `deploy/dev/matrix/`、`deploy/dev/core/compose.yaml`、`deploy/dev/core/Dockerfile`），
  `deploy/dev` 其余的只给本机开发栈（开发索引 `core/store/`、`experiments/`、smoke 脚本），改了不跑重作业；
  新的 `include_str!` 指向 `deploy/` 时要补进列表。只改 `.md` 的 PR 每个工作流都跳过（`apps/compose/` 下的也算）。
  新增依赖或测试输入时必须同步维护；定时和手动运行不按路径过滤。`rust-docker` 的路径范围写在它的 `push` 触发器上，
  `gate.yml` 的 `docker` 过滤复制同一份（PR 上由 `gate` 按它决定跑不跑），两处要一起维护。
- 草稿 PR 上 `changes` 和 `rust`、`ios`、`android`、`docker` 都不起，只有 lint 和汇总；lint 只跑冲突标记和密钥扫描（同一个作业的
  相邻两步）。转 ready（`ready_for_review`）按改动范围运行。
- PR 运行和 `main` 上的 `push` 运行都 `cancel-in-progress`：反正要重跑，旧的别继续占名额；定时运行跑完（组名单列）。`gate` 整轮取消时调用的重作业跟着取消；
  被调用的工作流自己的并发组名带前缀（`rust-`、`rust-docker-`、`ios-`、`android-`），和 `gate-` 不重名，否则 GitHub 判成调用方和
  被调用方互相等待。
- `cache.yml` 不取消进行中的运行，否则 main 忙时缓存几乎写不回去；并发组按作业分，同一作业只跑一个、只留最新一个等着。
- 合并进 main 不重跑快检查（规则集不要求 PR 先追上 `main`，两个各自绿的 PR 合在一起才出的错靠下面的定时兜底）；
  `push` 到 main 跑 `gate`（lint 和全量 rust；docker 由 `rust-docker.yml` 自己的 push 按路径跑）、`ios` 和 `android-size`
  （改到各自路径时）、main-image，`cache.yml` 用和 PR 作业一样的 key
  把编译缓存编一遍存下来（只编不跑）。每天定时（`schedule`，02:00 起错开）把 Rust、Docker、Android、iOS 和体积门禁
  全量跑一遍兜底。
- 打标签（`labeled`）不进 `ios.yml`、`android.yml`：那会用一次标签事件重跑整轮日常作业，而且跳过的那轮会把之前
  失败的必需检查盖成“通过”。重录统一走 `snapshots-record.yml`，只在标签是 `record-snapshots` 时跑一个作业，
  不算必需检查。
- `test-images.yml` 每周一 02:00 UTC 把测试和镜像构建要拉的上游镜像复制到 `ghcr.io/<owner>/test-images`
  （`scripts/mirror-test-images.sh` + crane copy，digest 原样保留），也可以手动触发。托管 runner 的
  出口 IP 拉不动 Docker Hub 和 ECR Public（实测 `registry:2` 返回 toomanyrequests、构建基础镜像 429），
  所以：Docker 集成测试作业设 `WONTON_TEST_IMAGES`，测试代码经 wonton-test-images crate 把镜像引用指到
  副本，本地不设、按上游引用拉；main-image 和 release 给 Dockerfile 的 `*_IMAGE` build-arg 传副本引用。
  仓库公开当天先手动跑一次 test-images.yml，再让 main-image 跑（副本空着时镜像构建会拉取失败）。
  wonton-core 的 `test_docker_check` 是普通单测，在 rust 工作流的测试分片里跑：Docker 测试文件里出现的
  镜像引用不在镜像清单里（典型是绕开 `wonton_test_images::image` 把引用直接传给 `ensure_image`），就当场失败
  并指出文件和镜像名，那是写法错误，不进重试。官方索引检查（`plugin check-index`）也用不到 Docker，在 rust
  的第一个测试分片里跑。Docker 集成测试作业开跑前一次拉齐镜像：单个镜像最多重试 3 次
  （5 s、15 s 退避），这是基础设施重试，测试本身不重试；预拉用带摘要的引用、测试建容器也用它，
  本机有就不再联网。还拉不到时整个作业失败，日志里是
  `::error::基础设施失败：拉不到镜像 <镜像>（重试 3 次）`，和断言失败分开。

## 界面参考图（Android）

- `apps/compose/shared` 的 host test 里有 Paparazzi 截图测试（Apache-2.0，版本钉在
  `gradle/libs.versions.toml`）。它按布局在 JVM 上把 `@Preview` 渲染成 PNG，不加载 Rust 的 .so；预览自己带
  固定假数据。参考图在 `apps/compose/shared/src/androidHostTest/snapshots/`，直接进 git，不用 LFS。
- 比对跑在 android 工作流的构建作业里（`verifyPaparazziAndroidMain`），不新开作业：同一个 PR 上的新检查放进
  已有作业，规则集的必需检查列表不用动。设备固定成 PIXEL_5、语言 en、浅色，另有深色的一份；参考图在 Linux
  runner 上生成，本机字体不同也可能有差异，以 CI 的为准。
- 比对失败时差异图和报告作为 `paparazzi-<运行次数>` 产物留 7 天，下载下来看差在哪。
- 重录：本地 `cd apps/compose && ./gradlew :shared:recordPaparazziAndroidMain` 后提交 `snapshots/` 下的
  PNG，和预览一起提交。
- 或者在 PR 上打 `record-snapshots` 标签，`snapshots-record` 工作流跑 `recordPaparazziAndroidMain`、
  把新图提交到分支头，然后摘掉标签。它是单独的工作流、不是必需检查：打标签不会起一整轮 android 构建，也不会
  取消正在跑的那轮，草稿 PR 上也照跑；重录这一轮红或跳过，不会把之前失败的必需检查盖成通过。录制和提交都由
  它做，新图怎么过一遍比对见 iOS 一节的“重录推回之后”。外部 PR 的分支
  在别人仓库里推不进去：标签照样摘掉，图放在这次的 paparazzi 产物里，作者下载后自己提交。
- 标签用完就摘：GitHub 不会给已经贴着的标签再发一次 `labeled`，留着就发不出下一轮重录。摘标签是单独一步、
  只看标签（外部 PR 也摘），不跟着提交步骤的成败走。

## Release APK 体积（`android-size.yml`）

`release-size` 要在 ubuntu 上编一份 release 核心，冷缓存十几分钟，比 PR 要等的快检查慢，所以不在 `gate` 里：
只按 `push` 到 main 的 android 路径、每天定时和手动触发。它出 arm64 Release 包并跑 `scripts/check-apk-size.sh`
（上限 53 MB），失败时把符号和 R8 映射表传成产物、留 14 天；成功不传（mapping.txt 一百多 MB，每次都存太占产物存储）。

## Release App 体积（`ios-size.yml`）

和 Android 的 `android-size.yml` 同一个角色：只按 `push` 到 main 的 iOS 路径、每天定时和手动触发，不在 `gate` 里。
它出 arm64 的 Release 归档（不签名）并跑 `scripts/check-app-size.sh`：打成 ipa 量下载大小、按位置列出各部分、
查主二进制还剩多少符号，任何一项超标就失败；失败时把 dSYM 和体积报告传成产物、留 14 天。上限取修后大小 +15%，
和 APK 同一个口径。Release 核心由 `cache.yml` 在 main 上预编（和 APK 一样），这边只增量链接。

## 汇总作业的条件

`rust.yml`、`ios.yml`、`android.yml`、`android-size.yml`、`ios-size.yml`、`rust-docker.yml` 的定时和手动运行不经过 `gate`，没有改动范围，全量运行。`gate` 里重作业的 `if`
以 `!cancelled()` 开头、再看开关；汇总用 `always()`，判断成功、跳过或失败都照常报告。范围判断失败会让汇总
变红，不会伪装成没改动。

## iOS 单测和截图测试

`ios.yml` 的“模拟器构建”作业用 `build-for-testing` 加 `test-without-building`，在固定型号
（`SIMULATOR_NAME`）和这台机器上挑出来的系统版本的模拟器上只跑 `WontonTests`，包括 `SnapshotTests`
（pointfreeco/swift-snapshot-testing，MIT，钉版本，只进测试目标不链接 App）。UI 测试在夜跑里跑，这个作业
不编。参考图在 `apps/ios/WontonTests/__Snapshots__/`，直接进 git，不用 LFS。

- 系统版本不写死：作业有 27.1 就用 27.1，没有用机器上最新的 iOS 版本（自托管 mini 没有 27.0，装运行时要
  sudo；托管镜像有）。`nightly-ui`、`snapshots-record` 同一个口径。换成另一个版本时参考图要打 record-snapshots 重录。
- 参考图绑定型号标识（`SIMULATOR_MODEL`，即测试进程里的 `SIMULATOR_MODEL_IDENTIFIER`）和系统版本，外观、
  语言（en、US）和时区（UTC）钉在 scheme 和测试里。换了型号或系统要整体重录，只换图不换参数会一直红。
  工作流是参考运行时的唯一来源：测试步骤用 `TEST_RUNNER_SNAPSHOT_MODEL`、`TEST_RUNNER_SNAPSHOT_OS` 把它交给
  测试进程（xcodebuild 只转发 `TEST_RUNNER_` 前缀），再加 `TEST_RUNNER_SNAPSHOT_ENFORCE=1`。测试按型号和
  系统版本核对，不符就整类失败，变量没交到也失败，不会出现改了工作流没人知道、CI 一路绿的情况。本机没有
  这些变量时只跳过，跳过原因里写清参考组合和当前组合。
- 每个用例默认 10 秒、最多 60 秒，失败不自动重试。
- 录制模式靠 scheme 的环境变量切：`Wonton` 是 `SNAPSHOT_TESTING_RECORD=never`（只比较，缺一张参考图也直接红，
  不在 CI 里悄悄补图），`WontonSnapshots` 是 `all`。xcodebuild 步骤的 `env` 传不进模拟器里跑的 App，只有 scheme
  的环境变量进得去。
- 比较失败时“失败时存测试结果包”一步把 `.xcresult` 存成产物，保留 7 天；新图和差异图是里面的附注。
- 重录在 `snapshots-record.yml`：在 PR 上打 `record-snapshots` 标签（草稿 PR 的事件不跑日常作业，但重录作业
  照跑）。作业用 `WontonSnapshots` scheme 把参考图全部重画，提交回分支并摘掉标签。录制那一轮本身是红的
  （每录一张报一条“Automatically recorded snapshot”），以提交回去的图为准；它不算必需检查。录完逐个核对
  现有参考图都出了“已录制”：运行时不对或一张没录就失败，不提交、不摘标签，留着看日志后重打。
- 重录推回之后：`GITHUB_TOKEN` 推的提交照常触发 `pull_request`，但仓库要求外部贡献者的运行先批准
  （`all_external_contributors`），github-actions[bot] 也算外部，`gate` 停在 `action_required`、
  分支头上没有必需检查。`snapshots-record.yml` 末尾的 `checks` 作业用同一个令牌批准这次运行；分支头已经
  不是重录推的提交（有人在那之后推过）时不处理。`workflow_dispatch` 重新触发不行：实测手动运行的检查不进
  PR 的检查汇总，规则集不认。外部 fork 的 PR 令牌推不回对方的分支：图存成 `ios-snapshots` 产物，作者自己取。
- 新增截图用 `WontonTests/SnapshotFixtures`（Debug 专用）搭界面，只放有代表性的界面，不逐页全录。

## 缓存

- 缓存主要写在 main 上，PR 和定时运行只读：仓库公开，外部 PR 写的条目会把 main 的挤掉。
  （核心产物是例外，PR 现编的那份只存在本分支作用域，见下面“核心产物”。）
  main 上不跑快检查（`rust`、`android` 只由 PR 上的 `gate` 和定时运行，`ios` 在 main 上只跑部分路径），所以另开 `cache.yml`：push 到 main 或手动触发时，用和 PR 作业**完全相同**
  的 rust-cache key 前缀、缓存目录和编译目标只编不跑（测试二进制、Clippy、iOS 模拟器核心、Android 核心、
  桥镜像）。改这几个工作流的 rust-cache 参数（`shared-key`、`workspaces`、环境变量）时要和 `cache.yml`
  一起改，少一处 PR 就恢复不到。手动触发也写缓存：换分支验证能不能恢复时用它。
- `save-if` 要写成 `${{ … }}`：不包的时候 runner 把表达式原样当字符串交给动作，动作只认字面量 `true`，
  结果一次也存不下来（之前就是没包，调度器日志里能看到传给动作的是 `github.ref == 'refs/heads/main'` 本身）。
- Rust 作业用 Swatinem/rust-cache，四个测试分片共用 `rust-test` 一个 key（编的是同一套测试二进制），
  “Rust Docker 测试”也恢复它；Clippy、iOS、Android 各有各的 key（iOS 是 macOS 产物，不能和 Linux 混）。
  测试用 cargo-nextest 按测试名哈希分四片并行；并发上限、卡死时限和 Docker 重活的分组在
  `.config/nextest.toml`。nextest 不跑 doctest，第一片里单列一步 `cargo test --doc`，`cache.yml` 也先把
  doctest 编好。
- 镜像构建的层缓存走 registry（`type=registry`，包是 `ghcr.io/<owner>/wonton-ci-cache`），不占 10 GB 的
  actions 缓存：main-image 每次合并写回，发版只读；桥镜像的预构建只读：层缓存只由 `cache.yml` 的桥镜像作业排队写回，
  `rust-docker` 的预构建和测试只恢复，构建完 `--load` 回本机给测试用。Dockerfile 用 cargo-chef 把依赖编进独立的依赖层：清单不变就不重编依赖，
  缓存要 `mode=max` 才存得下这一层。
- actions 缓存上限 10 GB，用它的有 rust-cache 和下面的核心产物缓存，超了按最久未用逐出。

## 核心产物（App 不再从头编核心）

只改壳（`apps/ios`、`apps/compose`）或文档的 PR 不编 Rust 核心，直接用 main 上同一份核心源码编好的产物：
iOS 是 XCFramework 和 Swift 绑定，Android 是 .so 和 Kotlin 绑定。键是对核心输入取的内容哈希，
只在 `scripts/core_artifacts/key.py` 一处算：

- 算进去的：`crates/**`、`vendor/**`（`.md` 不算，见下）、`.cargo/**`、`Cargo.toml`、`Cargo.lock`、
  `rust-toolchain.toml`、两个 `build-core.sh`、`generate-swift-bindings.sh`、Android 的 `gradle.properties`、
  `libs.versions.toml`、`shared/build.gradle.kts`，`CARGO_*`、`WONTON_*` 等相关环境变量，`.github/workflows` 里
  编产物的三个工作流（命令、cargo-ndk 版本、镜像、保存路径写在那里），探针取到的 Xcode 和模拟器 SDK 版本、
  工作流里钉死的 NDK 版本 `ANDROID_NDK_VERSION`（iOS 不看 NDK），以及键脚本自己。**新增会影响核心产物的输入必须加进去**，漏了壳会链接旧绑定。
- 已列出的输入有测试兜底：`scripts/core_artifacts/key_test.py` 对每个输入文件、每个输入目录、每项配方和探针都验
  “改了键必变”，同时验无关文件（Swift、Kotlin、`.md`、文档）不改键。改了源码类文件时 lint 跑它（纯 `.md` 和草稿 PR 上不跑）；
  测试只认已经列进 `key.py` 的输入，**新加一个会变产物的输入却忘了列入，测试抓不到**。
- 命中规则只有“同一个键”：`actions/cache` 精确匹配 `ios-core-<哈希>`、`android-core-<哈希>`，不带
  `restore-keys`。没命中就现编，编好存到同一个键上，不拿别的键的产物顶替；键算不出来（缺输入）时整步失败。
- 产物由 `cache.yml` 在 main 上存：`changes` 的 `core` 或对应的壳路径命中时现编一份存上，没命中时这条空跑。
  PR 现编的产物存在本分支的作用域里，同一个 PR 再推就命中；main 上存的条目所有 PR 都取得到。
- 取回来的产物先核对（iOS 要 `-simulator` 切片里的 `.a`、`Info.plist`、`module.modulemap` 和绑定 `.swift`；
  Android 要 `gradle.properties` 里每个 ABI 的 `.so` 和绑定 `.kt`），缺了当场停，不静默失败。
- 缓存条目 7 天不用会被逐出，这时下一次只改壳的 PR 会现编（只是慢），编完又重新存上。
- 产物只覆盖 CI 用的那份配方：模拟器 `aarch64-apple-ios-sim`、Debug、`arm64-v8a`，假数据入口随 Debug 打开。
  release 核心和 `x86_64` 的模拟器包由各作业自己编。
- 定时运行不取产物、强制现编：环境（rustup、Xcode、NDK、cargo-ndk）坏了和缓存条目坏了要每天有人发现。
  手动触发照常先取。

## 安全（仓库公开后对外部 PR 也会执行）

- 每个工作流顶层 `permissions: contents: read`，需要的作业单独放开（`packages: write` 推镜像、
  `packages: read` 拉私有的测试镜像副本）。在 main 上写层缓存用 `packages: write`，PR 上是 `packages: read`
  （外部 PR 的令牌到不了写）。
- 所有 `actions/checkout` 都带 `persist-credentials: false`，不在工作区留 git 凭据。
- 第三方 action 固定到提交 SHA（行尾注释写版本号）；SwiftLint、XcodeGen、gitleaks、crane、actionlint
  这类下载的二进制都校验 sha256。
- 不用 `pull_request_target`；artifact 只留测试输出，保留 7 天（夜跑的截图、录屏为查失败留 14 天，体积门禁失败时的符号和映射表也是 14 天）。

## 界面测试夜跑（`nightly-ui.yml`）

两端的界面测试每晚在模拟器上各跑一遍，只定时和手动触发，不挂 PR、不设必需检查。时间沿用
`ios.yml` 和 `android.yml` 的定时（一个工作流两条 cron，作业按 `github.event.schedule` 分开），不另占窗口。

- iOS 只开一台模拟器跑全部 `WontonUITests`；要两台的用例（粘贴的对方视角、好友两端）在工作流里排除或
  `XCTSkip`。要后端的用例没有配对码时 `XCTSkip`，不静默通过。手动触发可以填 `pairing_code`，把开发栈的
  配对码带进来（经 `TEST_RUNNER_PAIRING_CODE`，见 `apps/ios/scripts/ui-test.sh`）。
- 不需要后端的用例：全新安装落在配对页，和用 `WONTON_UI_FAKE` 假数据的首屏、侧边栏、主入口导航
  （`MainNavigationUITests`）、插件页的出口（`PluginExitUITests`）、面板网页的手势（`PanelGestureUITests`）、房间的通话入口（`CallEntryUITests`，
  Android 是 `CallEntryTest`）。假数据入口整个
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
- 规则集只有 `gate 汇总` 一个必需检查：被调用的作业改名、加作业都不动规则集；改动范围只看 `gate.yml` 的 `changes`。
