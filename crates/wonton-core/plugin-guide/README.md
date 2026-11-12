# 插件指南

`SKILL.md` 与 `assets/` 的两个例子是 `wonton__plugin_guide` 工具返回的说明和例子，编译进核心
（`src/mcp/authoring_tools.rs`）。**这份是源**；wonton-docs 的
[`skills/wonton-plugin/SKILL.md`](https://github.com/wontonapp/wonton-docs/blob/main/skills/wonton-plugin/SKILL.md)
与它保持一致，只保留链接差异：那边开头多一段指向[插件开发指南](https://github.com/wontonapp/wonton-docs/blob/main/develop.md)的说明。

改正文先改这边，编译进核心的那份随这个仓库的 CI 一起变；然后把 `SKILL.md` 和 `assets/` 复制到
wonton-docs 对应目录、补回那一段。改完用 `diff` 核对：两份 `SKILL.md` 的差异应只剩那一段。
