# Wonton 贡献规则

本文件对人和 AI 编程代理（Claude Code、Codex 等）同样适用。动手前先读
[docs/GOAL.md](docs/GOAL.md)、[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 和相关
[ADR](docs/adr/README.md)。

## 产品边界

- Wonton 是独立产品，不引入端脑云（Cephalon）的任何代码、账号体系或服务。旧 Lucy 仓库只作参考，
  不整段复制代码，也不导入其 git 历史。
- 服务端只支持“任意 Linux + Docker”，不做自有 OS 镜像、macOS 或 Windows 宿主。
- 核心保持薄：插件管理、网关与配对、推送代理、MCP 总线、密钥与存储授权。其余能力（包括 Agent、
  模型网关、相册）都是插件。

## 运行时原则：失败即停

- 生产路径不用 mock、预览数据、静默降级或兜底行为掩盖缺失能力。测试替身只能出现在测试代码里。
- 错误必须区分原因：传输不可用、权限缺失、插件未安装或未就绪、上游服务出错、输入非法。
- 状态不确定时如实显示为不确定，不伪装成功。

## 不造轮子

- 优先使用成熟的开源组件和开放标准（Matrix、iroh、MCP、ACP、OIDC、Subsonic、Compose）。
  要自研时，先在 PR 或 ADR 里写明现成方案为什么不行。
- 新增依赖要写明许可证。链接进我们二进制的代码禁止 AGPL/GPL；作为独立容器运行的插件
  （如 Immich、Navidrome）不受此限，只拉取上游官方镜像，不分发修改版。

## 代码结构

- CI 硬门禁：源码文件 ≤ 800 行，函数 ≤ 100 行，超过即失败。文件行数由
  `scripts/check-file-size.sh` 检查；函数长度由各语言 linter 检查（Rust clippy
  `too_many_lines`、Kotlin detekt `LongMethod`、SwiftLint `function_body_length`），代码落地时配置。
- 确需例外时写进 `.size-allowlist`，格式 `路径 | 原因 | 到期日 YYYY-MM-DD`；到期后 CI 重新失败。
- 按职责拆模块。新功能放进职责清晰的小模块，不往已经很大的文件末尾追加。
- 客户端遵循“厚 Rust 核心 + 薄原生壳”：状态、业务逻辑、网络、存储、ViewModel 都在 Rust；
  Swift/Kotlin 只渲染 ViewModel、把用户操作发回核心，以及调用平台能力（推送、相册、文件选择、
  后台音频等）。

## 测试

- 业务逻辑在 Rust 核心里，用行为测试覆盖：给定操作序列，断言 ViewModel 或对外副作用，不测实现细节。
- 核心 API 和插件清单用契约测试。
- 每个里程碑维护一条可在真实主机上复跑的端到端冒烟脚本。
- 不写快照测试、文案断言和只为凑覆盖率的测试；不设覆盖率门槛。
- 改了行为就改测试；删功能时连同测试一起删。

## 文档

- 只维护：README、本文件、`docs/GOAL.md`、`docs/ARCHITECTURE.md`、`docs/plugin-spec.md`、
  `docs/adr/`，以及各模块目录下简短的 README。
- 改变架构、协议、插件规范或用户可见行为的 PR，必须在同一个 PR 里更新对应文档。
- 重大决策写 ADR。推翻旧决策时新写一篇，并把旧 ADR 的状态改为“已被 ADR-xxxx 取代”。
- 不写进度报告、按日期的笔记和已完成的实施计划。进度看 GitHub issue 和里程碑。

## Git 与协作

- 主干开发：只有 `main` 一个长期分支。
- 一个 issue 对应一个分支、一个 PR；squash 合并后自动删除分支。
- 分支名用 `<type>/<简述>`，如 `feat/tunnel-http-proxy`。
- 提交信息用 Conventional Commits：`type(scope): 中文摘要`。`type` 取 `feat`、`fix`、`docs`、
  `test`、`refactor`、`chore`、`build`、`ci`；摘要说清行为变化，不写 `Update files` 这类空话。
- 发布打 tag（`vX.Y.Z`）；stable/beta 是镜像渠道，不是分支。
- 提交前运行 `git status --short` 和 `git diff --cached --stat`，确认只包含本次任务的改动。
  截图、录屏、`.xcresult` 等调试产物放在仓库外。
- 不提交密钥。测试用的示例配置放 `.env.example`。

## 多 Agent 并行开发

- 每个任务在独立的 git worktree 里做，分支从最新的 `main` 切出。
- 只并行彼此解耦的模块。要改跨模块接口（`crates/wonton-protocol`、插件规范）时，
  先单独提一个只改接口的 PR 合并，各任务再基于它开发。
- PR 描述写清改了什么、为什么、怎么验证（命令和结果）。
- 合并前 rebase 到最新的 `main` 并通过 CI。
