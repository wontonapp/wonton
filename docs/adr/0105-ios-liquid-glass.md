# ADR-0105 iOS 26 采用 Liquid Glass，旧系统使用材质，Android 保持 Material

- 状态：已接受（2026-11-13）
- 部分取代：[ADR-0029](0029-platform-material.md) 中“iOS 栏一律自排以保持 17 到 26 外观一致”“不采用 glassEffect”的选择，以及仅用 Material 的 iOS 导航/控制层外观；[ADR-0033](0033-plugin-tabs.md) 的插件出口胶囊淡黑底规定。
- 保留：ADR-0026 的颜色/间距/字体/图标令牌与内容层样式；ADR-0029 的无实底栏、滚动边缘与房间视口约束；ADR-0095 的无底栏、插件整页与唯一出口；ADR-0103 的宽屏布局边界。

## 背景

主人要求新 iOS 上的会话列表菜单与导航控件采用系统 Liquid Glass。维持所有系统版本外观一致不再是目标。

`safeAreaBar` 负责安全区与滚动边缘，自绘 plain Button 不会因此自动成为玻璃按钮。继续隐藏系统栏并给按钮画旧底，会错过系统外观、交互与无障碍适配。

## 决策

### 原生优先，不把内容层玻璃化

iOS 26 起，普通 NavigationStack 页面的导航与操作使用系统 toolbar；不覆盖其背景，不再用隐藏系统栏加自绘顶条模拟同一层。根页面不额外显示页面名，连接提示和动作保留。显示内容仍用共用令牌和 Phosphor 图标；平台容器控制栏的排版与材质。

系统 sheet、popover、contextMenu、照片/文件选择、分享和媒体控制保持系统呈现，不再叠自制玻璃。通用底部面板能表达已有内容与动作时采用系统 sheet；动作仍须在呈现消失后执行一次，取消不执行。

无法用原生容器表达的布局保留自绘：插件网页右上出口胶囊、手机推开式侧边栏、带回复/附件的聊天输入操作层、复杂错误详情卡。这些只在各自功能层的边界内应用一层 `glassEffect(.regular, in: ...)`；保留产品交互，不为玻璃把网页缩小、把抽屉改成手机分栏或把多步骤流程改成另一套导航。

普通气泡、卡片、列表行、表单、代码复制、开关和提示内容不套玻璃。插件连接/升级提示允许继续用普通 Material；不是有 Material 的地方都要变成 Liquid Glass。

### 按钮与组合

- toolbar 中的标准按钮让系统处理材质与反馈，不保留覆盖效果的旧底，不额外套 glassEffect。
- 独立导航/辅助悬浮按钮用 `.glass`；新聊天等主要悬浮动作用 `.glassProminent`。形状、前景和尺寸遵循现有令牌与可读性约束。
- 只有标准按钮样式无法表达的自绘可操作件才用 `.regular.interactive()`；大面板、被动说明和整个列表不 interactive。
- 同一功能簇存在多个独立自绘玻璃形状时使用 `GlassEffectContainer`。容器 spacing 取设计令牌，静止时不因过大 spacing 误融为一团；单个玻璃胶囊不额外包一层容器。
- 玻璃 shell 与子按钮不能各铺一层玻璃。形变确有产品含义时才使用 glassEffectID 等关联，不为了效果新增动画。

### iOS 17–18

最低版本保持 iOS 17。26 专属 API 的声明与调用均置于可用性边界内，旧系统不接触这些类型。

旧系统的自绘顶栏/悬浮控件继续使用 Material；新采用玻璃的自绘功能层提供相同形状、命中和动作的 Material 分支。系统 sheet 和菜单用该系统自己的材质。原来实底的复杂对话框、抽屉以及淡黑出口胶囊纳入 Material 分支，是这项决定的旧系统外观变化，不伪装成原本已有。

旧系统不是另一套业务逻辑：状态、动作、错误、剪贴板/输入法与网络路径相同，只有壳的呈现策略分支。不能为了统一底色给整页加不透明蒙层掩盖降级问题。

### 与滚动边缘、聊天视口、安全区共存

- 原生 toolbar 自带系统滚动边缘；不要再为同一条栏同时加 safeAreaBar，造成双重顶 inset。
- 自绘栏在 iOS 26 上用 safeAreaBar 接入滚动边缘，其内部需要玻璃的控件仍显式选择按钮样式或 glassEffect。更早系统保留 safeAreaInset + Material。
- 不垫满宽实底色，不按网页取色，不在玻璃上叠噪点。内容穿过普通页面栏的规则不变。
- 聊天房间保留 ADR-0029 的视口从顶栏下方开始，不用新的 ignoresSafeArea 把倒排消息塞到标题和状态栏下面。键盘/旋转/旁聊状态都须维持可读与可操作；玻璃是该边界之上的功能层，不是挪视口的理由。
- 插件整页继续按 ADR-0095 铺满，右上胶囊常驻，不增设一条完整顶栏，不重新引入底栏；宽屏列边界按 ADR-0103。
- 安全区的内容画布应连续，不能借 glassEffect 或 Material 又铺一条与页面断开的系统区色带。背景可以延伸，按钮与文字仍尊重安全区。已有聊天打磨修复先集成，本 ADR 不另写一套 inset/手势状态机。

### Android 对应

Android 按 ADR-0029 走 Material：顶栏是 TopAppBar/滚动表面，主要悬浮动作是 FloatingActionButton，底部呈现是 ModalBottomSheet，侧边栏/对话框/输入栏为令牌驱动的 Material 表面，按钮有平台按压反馈。

对应的是导航层次、动作与可读性，不是透明度或折射像素。无需引入实时模糊/玻璃库，不恢复底栏，不替普通内容全量更换控件。已满足的平台部件不为凑代码变更重写；没有对应的 iOS 材质差异在 PR 中逐项解释。

### 无障碍与验证约束

系统控件接管其材质对降低透明度/增强对比度的适配；自绘玻璃也必须用系统 API 并实测这些设置，不用自算 alpha 假装适配。令牌前景在深浅复杂背景上都必须可读。减少动态打开时不自行追加弹簧/形变。

读屏名字/角色/选中态、至少现有 token 命中区、大字号、键盘焦点与返回语义不能因新材质丢失。玻璃、阴影和模糊不是验收文字可读性的证据，必须看实际界面。

## 理由与代价

原生控件少维护一份系统行为，避免自绘底层盖掉系统效果；仅产品独有布局使用玻璃 API。iOS 版本之间允许不同外观，两套运行时都要验收。系统外观会随 SDK/系统变化，像素参考图绑定固定环境，升级需要重录而不是放宽容差。

## 验证结果

2026-11-13，以 `2f658e0` 加独立控件改动，在 iPhone 17 Pro / iOS 26.5、浅色主题、固定中文或英文、真实开发后端或无后端侧栏夹具条件下，完成了以下有界验证：

- `.glassProminent` 新聊天按钮构建成功；真实主入口走查修前/修后各 1 条、0 失败，覆盖列表、房间与键盘、侧栏开合、设置和应用网格。修后按钮可进入发起私聊页并返回，辅助功能树命中区为 56×56pt。
- 同一运行条件下，`SidebarButton` 的标准 `.glass` 构建成功；既有侧栏打开/收起及左边缘滑开用例 1 条、0 失败（13.345s），截图显示玻璃圆钮，辅助功能树命中区为 54×54pt。
- iOS 17/18 运行时未出现在授权 mini 环境，旧系统 Material 分支未做运行时外观验证；因此本 ADR 的实测结论不覆盖旧系统。

## 官方依据

[Adopting Liquid Glass](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass)、[Applying Liquid Glass to custom views](https://developer.apple.com/documentation/swiftui/applying-liquid-glass-to-custom-views)、[glassEffect](https://developer.apple.com/documentation/swiftui/view/glasseffect(_:in:))、[GlassEffectContainer](https://developer.apple.com/documentation/swiftui/glasseffectcontainer)、[glass](https://developer.apple.com/documentation/swiftui/primitivebuttonstyle/glass)、[glassProminent](https://developer.apple.com/documentation/swiftui/primitivebuttonstyle/glassprominent)、[interactive](https://developer.apple.com/documentation/swiftui/glass/interactive(_:))、[safeAreaBar](https://developer.apple.com/documentation/swiftui/view/safeareabar(edge:alignment:spacing:content:)-9hwgb)。
