# 复审 PR 833 iOS 加密推送（通知扩展与登记）

非作者复审，这一轮是 `rv-833c` 七条返修的验收。先读 `~/wonton-dev/tasks/fe-review-common.md`、PR #833 正文和 ADR-0003。

## 要核

1. **App 锁不露内容**（`rv-833c` 第 1 条）：核心在 `app_lock.rs` 把锁定标记写进组容器（`app-lock-locked.json`），
   扩展的 `PushLock.swift` 发通知前读；锁着时标题换成 App 名。核心用例 `app_lock::tests::*marker*`，扩展用例
   `PushLockTests`（读不懂当锁着）。
2. **推送权限和 UI 用例**（第 2 条）：`PushModel.start()` 在 `WONTON_DEV_SKIP_PUSH_AUTH` 下不问权限，
   `NotificationSettingsUITests` 用它验“还没问过”，同一台再跑按已允许走。实跑结果写在 PR 评论里。
3. **钥匙串失败**（第 3 条）：`Unavailability.keychain` 进设置页，Release 不再静默。
4. **切服务器**（第 4 条）：`AppNavigation.switchPushServer` 只在新设 rejection 时 break，旧对话框不再让它空跳。
5. 文档（第 5、6 条）：ADR-0003 的中继两行、ADR-0101 的过时段、ADR-0102 §4 的实现说明、探针注释。
6. **中继地址的构建配置**（第 7 条）：`project.yml` 的 `WontonPushRelay: ""` 和 README 的 `INFOPLIST_KEY_` 覆写，
   结论写在 PR 评论里；验不过就改成 xcconfig 默认值。

## 环境

代码在 npchy 的 `/home/npchy/wonton-dev/fix-833-snap`（分支 `fix/ios-push-lock`），构建发 mini（`ssh mini-ios`），
Swift 用例和 UI 用例在模拟器上跑。连栈的 UI 用例走 `mini-walkthrough.sh`。

## 结论

在 PR 上发评论，单独一行 `复审通过 <head 前 8 位> 复审：<你的卡名>` 或 `复审要改 <head 前 8 位> 复审：<你的卡名>`。
别 merge、别改产品代码、别打 tag。
