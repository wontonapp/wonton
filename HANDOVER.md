# 复审 PR 833 iOS 加密推送（通知扩展与登记）

非作者复审。先读 `~/wonton-dev/tasks/fe-review-common.md`、PR #833 正文和 ADR-0003。

## 要核

1. **RFC 8291 解密**：`PushDecryptTests` 用的向量和核心 `push/tests.rs` 是同一组；扩展只解密、不写业务状态；P-256 私钥和认证密钥只在 App Group 钥匙串。
2. **登记**：APNs 令牌 → 中继取地址 → 同一个 `SetPushEndpoint`；没有中继/没权限/中继不可达时设置页按类别给原因，不假装已开启；前台不发通知、离开后台前让核心检查连接。
3. **点开路由**：`push_server_slot` 按 endpoint 换服务器再进房间；服务器被移除时如实报，不是静默什么都不做。
4. **失败面**：解不开仍送一条通用通知；确认卡片用标题当正文（ADR-0047）。
5. **lint 红**（`lint / 冲突标记、密钥扫描与结构门禁`）先查清原因：是协议 base 没过就 rebase，是密钥扫描就把测试密钥标 `gitleaks:allow`；绿了才给结论。
6. 构建配置 `WontonPushRelay` 缺省时行为清楚；ADR-0003 的 iOS 段和实现对得上。

## 环境

代码在 1k，构建发 mini（`ssh mini-ios light <名> -- <命令>`，源码 `git archive HEAD | ssh … tar -xf -`）。Swift 测试在模拟器上跑。

## 结论

写 `~/wonton-dev/rv-833.result.md`，最后一行 `VERDICT: PASS`/`VERDICT: CHANGES`（带行号）。PASS 且 head 没变时评论 `复审通过 <head 前 8 位> 复审：rv-833`。别 merge、别改产品代码。

## 停

1.5MB 写 HANDOVER 停。head 变了 BLOCKED。
