# Wonton / 云吞

> 云吞，倒过来读是“吞云”：把原本放在云上的 AI、相册和网盘，吞回你自己家里。

Wonton 是一个以 Agent 为主入口的 AI 原生家庭服务器。它装在任意一台 Linux + Docker 主机上
（小主机、NAS、旧电脑都行），Mac、Windows 上由它自带的 Linux 虚拟机来跑。手机上的 Wonton App 像一个即时通讯软件：你的 AI 是第一联系人，
能听懂你、替你办事、主动给你发消息。相册、文件、音乐、下载、本地模型等一切能力都是插件，
Agent 通过工具调用它们；有网页界面的插件，在 App 里像小程序一样打开。

- **数据在家**：消息、文件和密钥只存在你自己的服务器上，我们运营的云端只转发密文。
- **随处可达**：App 内置 iroh 点对点隧道，用蜂窝网络也能直连家里，不需要公网 IP、端口转发或系统 VPN。
- **真正的 IM**：基于自托管 Matrix，多端同步、家人群聊、离线推送都是现成的。
- **万物皆插件**：插件就是一个带 `x-wonton` 扩展字段的 `docker-compose.yml`。
- **Agent 可替换**：默认 Hermes，也可以换成 Codex、Claude Code 等支持 Matrix 或 ACP 的 Agent。

## 文档

- [docs/GOAL.md](docs/GOAL.md)：总目标与各里程碑的验收标准
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)：系统架构、主要链路与选型
- [插件规范](https://github.com/geraldpeng6/wonton-docs/blob/main/plugin-spec.md)：`x-wonton` 清单（公开仓库）
- [docs/adr/](docs/adr/README.md)：架构决策记录
- [AGENTS.md](AGENTS.md)：贡献规则，人和 AI 编程代理都适用

## 仓库布局

```text
crates/      Rust 工作区：协议类型、iroh 隧道、服务端核心、客户端厚核心
apps/        iOS（SwiftUI）与 Compose（Android、桌面）原生壳
plugins/     示例插件与预装插件锁文件 system.lock；官方插件各在自己的仓库，见 ADR-0017
cloud/       推送中继、iroh 节点、签名静态文件的部署配置
deploy/      一行安装脚本、compose 模板、升级器
docs/        文档
scripts/     CI 与开发脚本
```

各目录用途见其 README，Rust crate 的说明见各自 `src/lib.rs` 的模块文档。

## 许可

源码公开供查看，版权所有，暂未授予使用、修改和分发的许可。

The source is published for viewing only. All rights reserved; no license to use, modify or distribute is granted yet.
