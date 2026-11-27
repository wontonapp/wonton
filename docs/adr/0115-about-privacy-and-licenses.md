# ADR-0115 关于页的隐私说明与构建期许可清单

- 状态：已接受（2026-11-24）

## 背景

“关于”原先只有版本号。主人要知道数据在哪、哪些环节会经过第三方（#661），App 的发布也要能回答“带了哪些第三方代码和许可”。

## 决策

### 1. 关于页加两个二级页

关于页 = 版本号 + “隐私” + “开源许可”，两页都返回关于页；两端都有。隐私说明写成短文，按标题分段，只写代码里核实过的行为（加密、推送、直连与中继、链接预览、插件登录、数据落盘与系统备份），四种语言进 Fluent，事实来源逐段挂在 PR 里。

### 2. 许可清单构建时生成，不手写

Rust 用 cargo-about 按 `licenses/about.toml` 的 accepted 收，Android 依赖用 AboutLibraries 导出，图标集（Kubai）不是依赖，按 `design/icons.toml` 钉的上游版本和许可加进去，每套单开一组、正文原样取 `design/<套>/LICENSE`（带上游的版权行），`scripts/generate-licenses.py` 合成一份 `licenses.json`；许可正文取仓库内 SPDX v3.27.0 快照，生成后校验结构、哨兵组件和表外许可。工具版本、资产和 sha256 钉死，按平台选下载资产；新工具是构建期依赖（MIT/Apache-2.0），不进二进制。

`licenses/APPROVED` 是唯一的许可结论：同一张表做 cargo-about 的 accepted 和 AboutLibraries 的 `strictMode = FAIL` 白名单，新增依赖带来表外许可（含 GPL/AGPL）构建失败，由人看过再补。

### 3. 解析在核心，两端只排

壳从 App 资源读 JSON 交给核心；分组、排序、四种错误（坏 JSON、空清单、缺全文、没有可显示的许可）都在 `crates/wonton-client/src/about/`，理由按 App 的语言给。分组键用许可 id，不用名字。

## 理由

- 构建时生成，代码里就能审到清单怎么来的；运行时不联网，也不依赖第三方界面。
- 一张白名单同时约束两个收集器，比两处各写一份强校验少一处漂移。
- 解析留在核心，iOS 没有 SPM 依赖可给 LicensePlist 读，也能用同一个生成器；壳不各写一遍 JSON。

## 代价与待验证

- 许可页没有渲染器：正文按等宽文本显示，长许可靠滚动；要署名（crate 自己的版权行）得另收 `license_file`，现在只有 SPDX 占位文本。
- cargo-about 的下载资产只覆盖 Linux x86_64 和 macOS arm64；其他平台要 `cargo install --version … --locked` 放进 PATH，生成器会报清楚。
- Paparazzi 参考图还没录；iOS 的许可页只在模拟器上走过查。
