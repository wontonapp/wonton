# ADR-0007 Agent 与 Bot：Agent 即插件

- 状态：已接受（2026-10-28）

## 背景

旧方案把 Hermes 当成唯一的 Agent，写了约 2.2 万行适配器和 2 万行测试，专门做执行轨迹、事件帧、
恢复状态等展示。新产品的形态更接近 MuseBot 这类个人 Agent：只收发消息，不展示详细的执行过程。
Agent 内核也要能替换，比如换成 Codex。

Hermes 上游已经自带 Matrix 等 20 多个 IM 平台、MCP、cron，以及 Bot Mode：一个 profile 就是一个
有独立模型、记忆、技能和定时任务的 Bot，多个 Bot 可以组群协作。

## 决策

- **Bot = 一个 Matrix 身份 + 一个驱动它的 Agent 插件。** 核心负责给 Bot 分配身份、权限、模型访问
  （magpie）和工具（MCP），不和任何具体 Agent 耦合。
- **接入标准**：
  - 会说 Matrix 的 Agent 直接接入，如 Hermes 的 Matrix 平台。
  - 支持 ACP（Agent Client Protocol）的 Agent 通过通用的“ACP 桥”插件接入，如 Codex、Claude Code、
    Gemini CLI、OpenCode，以及 Hermes 自带的 ACP 适配器。一个聊天房间对应一个 ACP 会话，ACP 的
    权限请求对应聊天里的确认按钮。
- **默认 Agent**：Hermes，走 Bot Mode，一个 profile 对应一个 Bot；只收发消息，不做执行轨迹和工具
  过程展示。
- **家人**：每个家人有自己的私人 Bot（独立记忆，互不可见）；主人可以建共享 Bot，拉进家庭群（M2）。
- **Bot 之间通信**：同机走 Matrix 私聊或群聊；跨机做成独立插件（Matrix 联邦走 iroh，或 A2A 协议）。
- **权限**：
  - 私人 Bot 只接受主人的指令。
  - 其他 Bot 要经主人同意才能加为联系人；跨机还要先配对两台服务器。
  - 来自其他 Bot 的消息一律视为不可信输入，涉及敏感工具必须由主人确认。
  - 设频率和转发跳数上限，防止 Bot 之间无限对聊。

## 理由

- Matrix 和 ACP 都是开放标准，一个桥就能接入一整类 Agent，不用为每个 Agent 写适配器。
- 只收发消息的形态大幅减少客户端和适配层的代码量。

## 范围

MVP 只有默认的 Hermes Bot。ACP 桥、家人 Bot、Bot 互发消息在 M2，接口现在预留；跨机通信在 M3。

## 代价与风险

- ACP 面向编辑器场景，桥需要为 Agent 提供受限的工作区（文件系统和终端能力）。
- 多个 Agent 同时运行会占用较多内存，资源少的设备（如 Pi 4GB）要限制数量。
