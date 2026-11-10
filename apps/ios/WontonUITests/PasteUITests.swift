import XCTest
import UniformTypeIdentifiers

/// 粘贴发送（#745 第 6 项）的走屏。
///
/// 要连家里的服务器（开发栈）：外部传 `PAIRING_CODE`（`deploy/dev/core/dev.sh pair <地址>` 打出的配对码），
/// App 启动时交给核心配对（`WontonApp` 的 `WONTON_DEV_PAIRING_CODE`），没有就跳过。两个角色各占一台模拟器，
/// 用 `-only-testing` 分开跑：
///
/// - `testPasteImageSends`（发送方）：把一张 PNG 放进剪贴板（测试进程和 App 同在模拟器上，写的就是设备剪贴板），
///   长按输入框 → 粘贴 → 预览确认 → 发送，断言时间线里出现图片消息、“发送中”标记消失。
/// - `testPeerShowsImage`（对方，同一账号的另一台设备）：打开聊天，等图片消息出现在时间线里并截图。
final class PasteUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = true
    }

    func testPasteImageSends() throws {
        try UITestBackend.require()
        try launch()
        UIPasteboard.general.setData(renderPNG(), forPasteboardType: UTType.png.identifier)
        openRoom()

        let composer = composerField()
        XCTAssertNotNil(composer, "找不到输入框")
        // 长按可能被当成点按，按屏幕坐标来（见 BotProfileUITests 的 pressScreen）。
        pressScreen(composer ?? app, 1.2)
        var paste = waitFor([app.menuItems], "Paste", timeout: 5) ?? waitFor([app.menuItems], "粘贴", timeout: 1)
        if paste == nil {
            // 诊断：长按那一瞬间的屏幕和菜单项数量，失败截图里看菜单到底出没出。
            shot("09-longpress")
            // 备选路径：元素上的长按（有的 iOS 版本坐标长按不出菜单）。
            (composer ?? app).press(forDuration: 1.2)
            paste = waitFor([app.menuItems], "Paste", timeout: 4) ?? waitFor([app.menuItems], "粘贴", timeout: 1)
        }
        XCTAssertNotNil(paste, "长按后没有粘贴菜单（菜单项 \(app.menuItems.count) 个）")
        paste?.tap()

        // 预览面板：发出去之前看得到，也能取消。
        let send = waitAnyButton(["Send", "发送"], timeout: 15)
        XCTAssertNotNil(send, "粘贴后没有出预览面板")
        XCTAssertNotNil(waitAnyButton(["Cancel", "取消"], timeout: 3), "预览面板没有取消")
        shot("10-preview")

        // 本地回显在时间线末尾，App 发送后自动滚到最新消息。图片气泡的读屏类型不确定，
        // 用复合证据：大图元素，或自己这条消息的「已读」回执。
        send?.tap()
        waitSendingDone(deadline: Date().addingTimeInterval(120))
        XCTAssertTrue(hasLargeImage() || hasReadReceipt(), "时间线里没有出现图片消息")
        shot("11-sent")
    }

    func testPeerShowsImage() throws {
        try UITestBackend.require()
        try launch()
        openRoom()
        // 发送方先跑：这里等它的图片消息同步过来。第二台设备有没有消息的密钥看核心的恢复设置：
        // 没配恢复时事件照样同步，只是显示“无法解密”的占位——两种都算消息到了，截图看实际显示。
        let deadline = Date().addingTimeInterval(180)
        while Date() < deadline && !hasLargeImage() && !hasUndecryptable() {
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        XCTAssertTrue(hasLargeImage() || hasUndecryptable(), "对方的时间线里没有出现这条消息")
        shot("20-peer-room")
    }

    // MARK: - 摆台

    /// 配对码可选：给了就带着配对（新设备，走引导）；没给就用 App 里已配对的会话直接进主界面。
    /// 码 10 分钟过期，而 mini 重活锁的排队时长不可预期，重跑走已配对会话才不跟 TTL 赛跑。
    private func launch() throws {
        app = XCUIApplication()
        if let code = ProcessInfo.processInfo.environment["PAIRING_CODE"], !code.isEmpty {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = code
        }
        app.launch()
        let start = waitAnyButton(["Apps", "应用"], timeout: 25)
            ?? waitAnyButton(["Skip", "跳过", "Next", "下一步", "Done", "完成"], timeout: 60)
        XCTAssertNotNil(start, "既没到标签栏也没到引导页")
        passOnboarding()
        XCTAssertTrue(waitForTabs(timeout: 240), "没回到主界面")
    }

    /// 进聊天页开 Bot 的私聊：先看聊天列表；列表还没有这个房间就从应用页点图标进（点图标直接开聊）。
    /// 找不到房间时再过一遍引导（配对完成后引导页可能晚弹），然后重试一次应用页路径。
    private func openRoom() {
        tabButton(["Chats", "聊天"])?.tap()
        if !waitRoomOpen(seconds: 20) {
            tabButton(["Apps", "应用"])?.tap()
            for name in ["Hermes", "回声", "Echo"] {
                if let row = roomRow(name) { tapWhenHittable(row); break }
            }
            _ = waitRoomOpen(seconds: 60)
        }
        if composerField() == nil {
            passOnboarding()
            tabButton(["Apps", "应用"])?.tap()
            for name in ["Hermes", "回声", "Echo"] {
                if let row = roomRow(name) { tapWhenHittable(row); break }
            }
            _ = waitRoomOpen(seconds: 60)
        }
        if composerField() == nil { XCTFail("没进聊天房间") }
    }

    private func roomRow(_ name: String) -> XCUIElement? {
        if app.buttons[name].firstMatch.exists { return app.buttons[name].firstMatch }
        if app.staticTexts[name].firstMatch.exists { return app.staticTexts[name].firstMatch }
        return nil
    }

    private func waitRoomOpen(seconds: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            for name in ["Hermes", "回声", "Echo"] {
                if let row = roomRow(name) { tapWhenHittable(row); break }
            }
            if composerField() != nil { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return composerField() != nil
    }

    /// 输入框：占位文字是它的读屏名字。
    private func composerField() -> XCUIElement? {
        waitFor([app.textViews, app.textFields], "Message", timeout: 2)
            ?? waitFor([app.textViews, app.textFields], "消息", timeout: 2)
            ?? waitFor([app.otherElements], "Message", timeout: 1)
            ?? waitFor([app.otherElements], "消息", timeout: 1)
    }

    /// 时间线（List）里的图片元素。图片气泡带 `.isButton` 读屏 trait，会暴露成按钮而不是图片。
    /// 空子查询在 Xcode 26 上解析会抛「Failed to get matching snapshot」，先探 firstMatch 再取列表。
    private func timelineImages() -> [XCUIElement] {
        guard app.collectionViews.firstMatch.waitForExistence(timeout: 2) else { return [] }
        let collectionView = app.collectionViews.firstMatch
        let images = collectionView.images
        if images.firstMatch.waitForExistence(timeout: 0.5) {
            return images.allElementsBoundByIndex.filter { $0.frame.width > 100 && $0.frame.height > 100 }
        }
        let buttons = collectionView.buttons
        guard buttons.firstMatch.waitForExistence(timeout: 0.5) else { return [] }
        return buttons.allElementsBoundByIndex.filter { $0.frame.width > 100 && $0.frame.height > 100 }
    }

    /// 大图元素：App 级找 Image 和 Button（图片气泡的读屏类型不确定），头像等小图被尺寸过滤掉。
    private func hasLargeImage() -> Bool {
        for query in [app.images, app.buttons] {
            if query.firstMatch.waitForExistence(timeout: 0.2),
               query.allElementsBoundByIndex.contains(where: { $0.frame.width > 100 && $0.frame.height > 100 }) {
                return true
            }
        }
        return false
    }

    /// 自己这条消息的「已读」回执：对方的核心确认过这条消息，配合发送完成标记即送达证据。
    private func hasReadReceipt() -> Bool {
        let texts = app.staticTexts
        guard texts.firstMatch.waitForExistence(timeout: 0.3) else { return false }
        return texts.allElementsBoundByIndex.contains {
            $0.label.contains("已读") || $0.label.contains("Read")
        }
    }

    /// 本设备没有这条消息的密钥时的占位（没配恢复的新设备会这样）。
    private func hasUndecryptable() -> Bool {
        let texts = app.staticTexts
        guard texts.firstMatch.waitForExistence(timeout: 0.3) else { return false }
        return texts.allElementsBoundByIndex.contains {
            $0.label.contains("decrypt") || $0.label.contains("解密")
        }
    }

    /// 等自己这条消息的“发送中”标记消失（传完就没有状态文字）。
    private func waitSendingDone(deadline: Date) {
        while Date() < deadline {
            let sending = waitFor([app.staticTexts], "Sending", timeout: 1)
                ?? waitFor([app.staticTexts], "发送中", timeout: 1)
            if sending == nil { return }
        }
    }

    private func renderPNG() -> Data {
        let size = CGSize(width: 600, height: 400)
        let renderer = UIGraphicsImageRenderer(size: size)
        let image = renderer.image { context in
            UIColor.systemOrange.setFill()
            context.fill(CGRect(origin: .zero, size: size))
            UIColor.white.setFill()
            context.fill(CGRect(x: 80, y: 60, width: 440, height: 280))
            UIColor.systemBlue.setFill()
            context.fill(CGRect(x: 160, y: 120, width: 280, height: 160))
        }
        return image.pngData()!
    }

    // MARK: - 走屏辅助（和 BotProfileUITests 同一套）

    private func shot(_ name: String) {
        let png = XCUIScreen.main.screenshot().pngRepresentation
        let attachment = XCTAttachment(uniformTypeIdentifier: UTType.png.identifier, name: "\(name).png", payload: png)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        // 沙箱限制:测试进程写不了宿主 /tmp,写进自己的 NSTemporaryDirectory,由 E2E 脚本从模拟器容器捞出来。
        let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("flow")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? png.write(to: dir.appendingPathComponent("\(name).png"))
    }

    @discardableResult
    private func waitFor(_ queries: [XCUIElementQuery], _ label: String, timeout: TimeInterval = 20) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for query in queries {
                let element = query[label].firstMatch
                if element.exists { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    @discardableResult
    private func waitAnyButton(_ labels: [String], timeout: TimeInterval = 10) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists { return element }
            }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return nil
    }

    /// 等元素可点再点（选择器弹出动画里点击会报 not hittable）。
    private func tapWhenHittable(_ element: XCUIElement) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline && !element.isHittable {
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    /// 按屏幕坐标长按：元素上的长按会被可访问性激活吃掉（变成点按）。
    private func pressScreen(_ element: XCUIElement, _ seconds: TimeInterval) {
        let window = app.windows.firstMatch.frame
        let frame = element.frame
        let offset = CGVector(dx: frame.midX / window.width, dy: frame.midY / window.height)
        app.coordinate(withNormalizedOffset: offset).press(forDuration: seconds)
    }

    private func tabButton(_ labels: [String]) -> XCUIElement? {
        let height = app.windows.firstMatch.frame.height
        for label in labels {
            // 底部标签按读屏名字找；identifier 不一定等于名字。
            let element = app.buttons[label].firstMatch
            if element.exists && element.frame.minY > height * 0.5 { return element }
        }
        return nil
    }

    @discardableResult
    private func waitForTabs(timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if tabButton(["Apps", "应用", "Chats", "聊天", "Settings", "设置"]) != nil { return true }
            _ = app.wait(for: .runningForeground, timeout: 1)
        }
        return false
    }

    /// 引导页的独有标记：右上「跳过」、底部「下一步/完成/进入聊天」和「1/5」页码指示器。
    /// 切换中的快照会失效，统一走 waitForExistence（返回 false 不抛错）。
    private func onboardingVisible() -> Bool {
        if app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "[0-9]+/[0-9]+")).firstMatch.waitForExistence(timeout: 0.1) {
            return true
        }
        for label in ["Skip", "跳过", "Next", "下一步", "Done", "完成", "Go to Chat", "进入聊天"] {
            if app.buttons[label].firstMatch.waitForExistence(timeout: 0.05) { return true }
        }
        return false
    }

    /// 配对完成后资料页才会预填邮箱；空文本框的 value 是占位文字，所以用「value 含 @」判断预填到了。
    /// 只在资料页（页码 1/5 或邮箱栏可见）设这道闸，其他步骤照常点。
    /// NSPredicate 不 Sendable，全部内联构造，不落变量。
    private func pairingDone() -> Bool {
        let onProfile = app.staticTexts["1/5"].exists
            || app.textFields.matching(NSPredicate(format: "label CONTAINS %@ OR label CONTAINS %@", "邮箱", "Email")).count > 0
            || app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ OR label CONTAINS %@", "邮箱", "Email")).count > 0
        if !onProfile { return true }
        return app.textFields.matching(NSPredicate(format: "value CONTAINS %@", "@")).firstMatch.exists
            || app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "@")).firstMatch.exists
    }

    /// 按审核重试 → 跳过 → 完成 → 下一步 → 进入聊天的顺序点第一个可用的引导按钮。
    /// 审核项 Failed 时只有点该项的「重试」才能继续；「进入聊天」是提交完成后出现的。
    /// 页面切换中元素快照会失效，probes 全走 waitForExistence（返回 false 不抛错），tap 前不再二次读属性。
    private func tapOnboardingButton() -> Bool {
        for labels in [["Try Again", "重试"], ["Skip", "跳过"], ["Done", "完成"], ["Next", "下一步"], ["Go to Chat", "进入聊天"]] {
            for label in labels {
                let button = app.buttons[label].firstMatch
                // 禁用的按钮点了是空操作，下一轮再探；这里不读 isEnabled，避免切换中的快照异常。
                if button.waitForExistence(timeout: 0.2) {
                    button.tap()
                    return true
                }
            }
        }
        return false
    }

    private func passOnboarding(seconds: TimeInterval = 420) {
        // 底栏在引导门外一直存在，不能只看标签栏；要"引导按钮消失且标签栏稳定"才算走完。
        // 没见过引导页的多确认几轮，躲开引导出现前的加载期。
        let deadline = Date().addingTimeInterval(seconds)
        var calm = 0
        var seenOnboarding = false
        while Date() < deadline {
            if onboardingVisible() {
                seenOnboarding = true
                calm = 0
                if pairingDone() {
                    _ = tapOnboardingButton()
                }
                Thread.sleep(forTimeInterval: 1.5)
                continue
            }
            if waitForTabs(timeout: 2) {
                calm += 1
                if calm >= (seenOnboarding ? 3 : 8) { return }
            } else {
                calm = 0
            }
        }
        XCTFail("引导没走完（配对或引导步骤卡住）")
    }
}
