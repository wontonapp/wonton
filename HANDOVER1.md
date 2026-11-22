# 接手 #857：快甩注入在 420dpi 上要绿，产品速度 ×3 也要绿

上一个会话（4.9MB）停了 14 小时，卡上还写着干活。工作区干净，在 `fix/android-roompanel-nightly` 的 552984eb 上，`.work/prev-attempt.diff` 是上一轮没落地的尝试，先读它再决定留不留。

## 要过的验收（复审写的，照做）

1. 快甩的注入要在大屏/高密度上站得住：1080×2400 420dpi 上现在稳红（`RoomPanelGestureTest` 272/273 行），旧的 `swipeWithVelocity` + 写死 2000 在同一台 420dpi 上通过，是本 PR 引入的。
2. 产品 `flickVelocity` 调到 3 倍时也要绿；门槛随之抬到约 2362 px/s。
3. 实测末速只有名义值的 0.2–0.7（顶部下拉 1750/3687、条带 978），别拿名义值当实测。

复审给的依据和细节：`~/wonton-dev/fe-review-857.result.md`（最后一段“必须改”）。

## 方向（复审的建议，不是死的）

- 顶部那条要末速：别用节点局部坐标算，按屏幕坐标注入；或者给到令牌的 3–4 倍并核对实测值。
- 条带那条行程本来就比 pullDistance 长（108dp），由距离承担，速度一栏是空的，别硬塞。

## 跑法

- 模拟器 fe-nightly-android-roompanel（已在跑，端口 5564）；用完 `~/wonton-dev/emu-slot.sh down fe-nightly-android-roompanel`（注意拼写按实际脚本）。
- 单设备跑：`ANDROID_SERIAL=127.0.0.1:5564 ... gradle :androidApp:connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=app.wonton.android.RoomPanelGestureTest`，走 orchestrator 那条路（工程里已配 execution=ANDROIDX_TEST_ORCHESTRATOR、clearPackageData）。
- 每改一版在 420dpi 和 320×640 mdpi 两台都跑；再临时把 flickVelocity 乘 3 跑一遍（改回原值提交）。
- 结论写 `~/wonton-dev/fe-nightly-android-roompanel.result.md`，最后一行 `READY <提交号>`。

## 停

- 工作区超过 1.5MB：进度写 `.work/HANDOVER.md` 停。
- 模拟器坏了、三台并发抢挂包管理：写 BLOCKED 停。
- 别改产品阈值（`flickVelocity` 本身）来让测试好过，除了临时 ×3 核对；别 push，别 merge。
