# Wonton 贡献规则

本文件对人和 AI 编程代理（Claude Code、Codex 等）同样适用。动手前先读
[docs/GOAL.md](docs/GOAL.md)、[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 和相关
[ADR](docs/adr/README.md)。

## 产品边界

- Wonton 是独立产品，不引入其他产品的代码、账号体系或服务。别的仓库只作参考：不整段复制代码，
  也不导入其 git 历史。
- 服务端是“任意 Linux + Docker”。Mac、Windows 上由云吞自带一台隐藏的 Linux 虚拟机来跑，核心和插件照样在 Linux + Docker 里，
  不做原生的 macOS 或 Windows 服务端（[ADR-0062](docs/adr/0062-desktop-vm-host.md)）。
- 不做自有 OS 镜像，只在 Mac、Windows 上带一台不可变的 Linux 虚拟机，系统盘整盘替换。
- 核心保持薄：插件管理、网关与配对、推送代理、MCP 总线、密钥与存储授权。其余能力（包括 Agent、
  模型网关、相册）都是插件。
- 架构遵循 ADR-0015：一个组件只做一件事，组件之间只经标准接口连接（Matrix、MCP、ACP、OIDC、Compose），
  每个组件都能单独替换；核心不为某个具体插件或 Agent 做特殊处理。

## 运行时原则：失败即停

- 生产路径不用 mock、预览数据、静默降级或兜底行为掩盖缺失能力。测试替身只能出现在测试代码里。
- 错误必须区分原因：传输不可用、权限缺失、插件未安装或未就绪、上游服务出错、输入非法。
- 状态不确定时如实显示为不确定，不伪装成功。

## 兼容

- 直到有第一个外部用户（主人以外的人装上）为止，协议、ViewModel、配置、插件规范都直接改，不写迁移、不留旧字段。
- 保留通用的向前兼容：核心和 App 分开升级时，不认识的按钮不显示，不认识的取值落到 `Other`。

## 不造轮子

- 优先使用成熟的开源组件和开放标准（Matrix、iroh、MCP、ACP、OIDC、Subsonic、Compose）。
  要自研时，先在 PR 或 ADR 里写明现成方案为什么不行。
- 新增依赖要写明许可证。链接进我们二进制的代码禁止 AGPL/GPL；作为独立容器运行的插件
  （如 Immich、Navidrome）不受此限。
- 插件优先拉上游官方镜像；上游没有镜像时，可以装上游的发布物（npm、deb 等），或从上游固定的版本或提交原样构建镜像。
  都不改上游源码、固定版本，不分发修改版。AGPL 的插件在插件页写明源码地址。

## 代码结构

- 源码文件按代码行数，不算空行和只有注释的行：超过 800 行警告，超过 2000 行 CI 失败，由
  `scripts/check-file-size.sh` 检查。函数 ≤ 100 行，超过即失败，由各语言 linter 检查：Rust 用 clippy
  `too_many_lines`，Kotlin 用 detekt `LongMethod`，Swift 用 SwiftLint `function_body_length`。
- 不许为了缩短行数写难懂的代码：删注释、把多行挤成一行、把穷举的 match/when 改成 `_`、缩短有意义的名字都不行。
  超过 800 行的警告是“该按职责拆”的信号，拆要按职责，不是挪几行凑数。
- 确需超过 2000 行时写进 `.size-allowlist`，格式 `路径 | 原因 | 到期日 YYYY-MM-DD`；到期后 CI 重新失败。
- 按职责拆模块。新功能放进职责清晰的小模块，不往已经很大的文件末尾追加。
- 客户端遵循“厚 Rust 核心 + 薄原生壳”：状态、业务逻辑、网络、存储、ViewModel 都在 Rust；
  Swift/Kotlin 只渲染 ViewModel、把用户操作发回核心，以及调用平台能力（推送、相册、文件选择、
  后台音频等）。

## 界面设计

- Avoid unnecessary secondary labels and descriptions. Let the layout do the explaining.
  分组、顺序、位置和图标已经说明了的，不再加副标题、说明文字或标签重复一遍。App 的原生界面和插件的网页都适用。
- 错误、权限和来源（如“未审核”）的提示不算多余，照常写清楚（见“运行时原则：失败即停”）。
- 加载和出错的界面写给普通用户看：一句人话说发生了什么、能做什么，配重试；仓库名、超时秒数、错误码收进默认收起的“详情”；
  部分失败是一条轻提示。类别由核心给，文字按类别写在壳里，不伪装成功、不藏错误。
- 界面文字放进各平台的本地化资源（iOS 的 String Catalog、Android 的 `strings.xml`），四种语言齐全，不在代码里写死或拼句子；
  核心只给类别和原始值，“详情”由核心按 App 的语言写。见 [ADR-0043](docs/adr/0043-app-languages.md)。
- App 的界面走 [ADR-0026](docs/adr/0026-design-system.md) 的设计系统，哪些共用、哪些按平台走以
  [ADR-0029](docs/adr/0029-platform-material.md) 为准。

## 开发环境与检查

- Rust 工具链版本由 `rust-toolchain.toml` 固定，所有 crate 在 `crates/` 下，同属一个 workspace。
- git hook 用 lefthook（MIT，单个可执行文件，只是开发工具；`brew install lefthook`）。`bash scripts/check.sh` 开头会装上（幂等），
  只想装 hook 用 `bash scripts/install-hooks.sh`。提交前自动 rustfmt、SwiftLint 修正并 stage，行数门禁和界面文字检查不过就拦下；
  推送前对改到的 crate 跑 Clippy，改到 `apps/ios/`、`apps/compose/` 才跑 SwiftLint、detekt，本机没装的工具打印“跳过”，由 CI 兜底。
  规则只在 `lefthook.yml` 一处，CI 用同一份配置查全部文件。不要用 `--no-verify` 绕过。
- 提交前运行 `bash scripts/check.sh`（行数门禁、冲突标记检查、格式、Clippy、测试），和 CI 的 Rust 部分一致；Swift、Kotlin 的检查在各自的工作流里。
- 依赖 Docker 的测试标 `#[ignore = "需要 Docker，…"]`。`check.sh` 连得上 Docker 就一起运行，连不上就跳过并打印手动运行的
  命令。跳过不等于通过：PR 说明里写明没跑。CI 分两个作业：“Rust”跑默认的测试，故意让 Docker 不可用，保证它们不依赖
  Docker；“Rust Docker 测试”运行这些标了 `#[ignore]` 的，Docker 不可用即失败。
- CI 用 `plugin check-index` 检查官方索引（wonton-store）里的每个清单这份核心都读得懂、都渲染得出来（不再注入的变量当场失败）。
  改清单写法或注入变量的 PR 要先让索引换成新核心读得懂的写法，否则 CI 失败。
- 没有 `/var/run/docker.sock`（OrbStack、Colima 等）时，`check.sh` 从当前 Docker context 取 `DOCKER_HOST`；
  直接运行 `cargo test` 要自己设置，testcontainers 不读 context。
- 依赖：只有一个 crate 用到的依赖写在该 crate 的 `Cargo.toml`；多个 crate 共用的依赖在根
  `Cargo.toml` 的 `[workspace.dependencies]` 统一版本，crate 里用 `{ workspace = true }` 引用。
- `Cargo.lock` 冲突时，先取 `main` 的版本，再运行 `cargo build` 重新生成，不手工合并。
- 本地开发环境（Tuwunel、示例插件等）放在 `deploy/dev/`，用 Docker 运行。

## 测试

- 业务逻辑在 Rust 核心里，用行为测试覆盖：给定操作序列，断言 ViewModel 或对外副作用，不测实现细节。
- 核心 API 和插件清单用契约测试。
- 每个里程碑维护一条可在真实主机上复跑的端到端冒烟脚本。
- 不写文案断言和只为凑覆盖率的测试；不设覆盖率门槛。
- Rust 侧不写快照测试。原生壳（Swift、Kotlin）允许截图测试：截图只从 SwiftUI 预览、Compose `@Preview`
  出，用固定的假 ViewModel 数据（测试替身只在测试代码里，见“失败即停”），参考图直接进 git，
  不用 LFS（公开仓库的 LFS 流量超了要收费）。截图测试只比像素，不必给每个预览都录。
- 界面改了不手改参考图：给 PR 打标签，由 CI 重录并提交，见 [.github/workflows/README.md](.github/workflows/README.md)。
- 能不能走通交给 CI（单测、截图测试、夜跑的界面测试），人工走查只留给视觉和手感上的改动。
- 改了行为就改测试；删功能时连同测试一起删。

## 文档与注释

文档和注释只记录读代码得不到的信息。会变的状态写进文档就会过时，过时的文字会把人和 AI 都带偏。

- 只维护：README、本文件、`docs/GOAL.md`、`docs/ARCHITECTURE.md`、`docs/glossary.md`（术语表）、公开仓库里的插件规范、
  `docs/adr/`，以及各模块目录下简短的 README。
- 改变架构、协议或用户可见行为的 PR，必须在同一个 PR 里更新对应文档。
- 插件规范在公开仓库 [wonton-docs](https://github.com/geraldpeng6/wonton-docs)，插件作者要看，主仓库是私有的。改规范的核心 PR
  同时在那边提 PR，核心合并后再合。那边是公开文档，不引用 ADR 和里程碑。
- 重大决策写 ADR，写法见 [docs/adr/README.md](docs/adr/README.md)。
- 不写进度报告、按日期的笔记和已完成的实施计划。进度看 GitHub issue 和里程碑。
- 注释只写代码表达不了的：为什么这样写（包括没选的写法）、约束和不变量、外部系统的实测行为。不写复述
  名字或类型的注释；导出给 Swift、Kotlin 的类型可以留一句说明，它会成为那边的文档。
- 文档和注释不写会变的当前状态（“现在”“还没”“目前”）：状态以代码、测试和 issue 为准。验证结果写明
  验证时的版本和条件。
- 一件事只写一处，别处放链接。改一项行为时，搜一下仓库里其他提到它的地方，一起改掉或删掉。
- 能写成测试的约定（接口字段、错误类别、安全约束）优先写成测试：约定被打破时测试会失败，文档不会。

## Git 与协作

- 主干开发：只有 `main` 一个长期分支。
- 一个分支一个 PR；squash 合并后自动删除分支。
- 一个 issue 可以分几个 PR：最后一个写 `Closes #N`，合并时 GitHub 自动关掉 issue；之前的写 `Refs #N`，并在
  issue 里留言说明还剩什么。不写的话 issue 不会自己关，做完的和没做的混在一起。
- 合并走 GitHub 合并队列：PR 的必需检查绿了，`gh pr merge --squash` 把它放进队列；队列把它和排在前面的 PR 合在一起，
  在最新的 `main` 上再跑一遍检查，绿了才真正合进去，红了把它踢出队列。不要求 PR 自己先追上 `main`。
- 必需检查在每个 PR 上都要有结果：工作流不在触发器上按路径过滤，由 `changes` 作业判断改动范围，没改到的重作业报 skipped。
  新增必需检查时同步改规则集。
- `main` 每次合并后都跑 CI。红了，谁合的谁马上看：是 PR 引起的，30 分钟内修不好就 revert 那次合并让 `main` 回绿，再在新 PR
  里修；是环境问题，修环境后重跑。`main` 红着、原因没查清时不合别的 PR。
- 时好时坏的测试修测试本身或它依赖的环境，不加自动重试：重试会把真问题一起盖住。
- 分支名用 `<type>/<简述>`，如 `feat/tunnel-http-proxy`。
- 提交信息用 Conventional Commits：`type(scope): 中文摘要`。`type` 取 `feat`、`fix`、`docs`、
  `test`、`refactor`、`chore`、`build`、`ci`；摘要说清行为变化，不写 `Update files` 这类空话。
- 发布打 tag（`vX.Y.Z`，1.0 之前是 `v0.x.0`，不带预发布后缀）；`stable` 是镜像渠道，不是分支。每次合进 main 还会推核心镜像 `:main` 和
  `:sha-<提交号>`（`.github/workflows/main-image.yml`），给插件仓库的 CI 跑端到端冒烟，不是发版。
- 提交前运行 `git status --short` 和 `git diff --cached --stat`，确认只包含本次任务的改动。
  截图、录屏、`.xcresult` 等调试产物放在仓库外（截图测试的参考图除外）。
- 不提交密钥。测试用的示例配置放 `.env.example`。
- 公开仓库（wonton-docs、各插件仓库、wonton-store）的提交、PR 和截图里，不带真实地址、主机名和内部渠道名。
  示例地址用 `192.0.2.10`。

## 多 Agent 并行开发

- 每个任务在独立的 git worktree 里做，分支从最新的 `main` 切出。
- 只并行彼此解耦的模块。要改跨模块接口（`crates/wonton-protocol`、插件规范）时，
  先单独提一个只改接口的 PR 合并，各任务再基于它开发。
- PR 描述写清改了什么、为什么、怎么验证（命令和结果）。
