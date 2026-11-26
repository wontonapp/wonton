import XCTest
import UIKit

/// #689 定点走查（一次性，不随产品提交）：加好友入口的取址引导、占位文字完整、错误类别与好友成功。
/// 每个测试现取一个新的配对码；同类文件按字母序，码也按这个顺序分。
/// 联系人是同服的普通家人账号（`contact689`，显示名 `689 Contact`），资料由核心设置。
private enum L {
    static let settings = ["Settings", "設定"]
    static let appearance = ["Appearance", "外観"]
    static let back = ["Back", "戻る"]
    static let dark = ["Dark", "ダーク"]
    static let friends = ["Friends", "友だち"]
    static let add = ["Add Friend", "友だちを追加"]
    static let send = ["Send Request", "リクエストを送信"]
    static let myCode = ["My QR Code", "自分のQRコード"]
    static let details = ["Details", "詳細"]
    static let retry = ["Try Again", "再試行", "重试", "重試"]
    static let invalid = ["That is not a Matrix ID", "Matrix ID ではありません"]
    static let forbidden = ["The request was refused", "リクエストが拒否されました"]
    static let upstream = ["Something went wrong", "エラーが発生しました"]
    static let waiting = ["Waiting for them to accept", "相手の承認待ち"]
    static let placeholder = ["Paste an address or link", "アドレスかリンクを貼り付け"]
    static let contactName = "689 Contact"
    static let noGroups = ["No groups", "グループなし"]
    static let contactUser = "@family1:"
    static let help = [
        "Ask them for their full friend address; in Wonton, once the Friends page loads successfully, they can copy their link from Add Friend → My QR Code.\nFormat only, not a real account: @bob:example.org",
        "相手に完全な友だちアドレスを送ってもらいましょう（Wontonでは、友だちページの読み込みが成功した後、「友だちを追加 → 自分のQRコード」からリンクをコピーできます）。\n形式の例（実在のアカウントではありません）：@bob:example.org",
    ]
}

/// 共用导航、等待和断言。
class Friend689Base: XCTestCase {
    nonisolated(unsafe) static var nextCode = 0
    var app: XCUIApplication!
    var japanese: Bool { false }
    var prefix: String { japanese ? "ja" : "en" }

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        let codes = try UITestBackend.codes()
        let index = Self.nextCode
        Self.nextCode += 1
        XCTAssertLessThan(index, codes.count, "配对码不够")
        app = XCUIApplication()
        app.launchArguments = japanese
            ? ["-AppleLanguages", "(ja)", "-AppleLocale", "ja_JP"]
            : ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_UI_TEST_STATE"] = "1"
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = codes[index]
        XCUIDevice.shared.appearance = .light
        addTeardownBlock { XCUIDevice.shared.appearance = .light }
        UITestBackend.launch(app)
    }

    func any(_ labels: [String], timeout: TimeInterval = 15) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.buttons[label].firstMatch
                if element.exists && element.isHittable { return element }
            }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return nil
    }

    /// 带数字的行走按钮的 label 是“Friends, 3”，按前缀找。
    func anyPrefix(_ prefixes: [String], timeout: TimeInterval = 15) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        var matched = false
        while Date() < deadline {
            matched = false
            for element in app.buttons.allElementsBoundByIndex
            where element.exists && prefixes.contains(where: element.label.hasPrefix) {
                if element.isHittable { return element }
                matched = true
            }
            // 行在列表/抽屉下面看不见时贴着它往下滚一屏再找。
            if matched, let hidden = app.buttons.allElementsBoundByIndex.first(where: {
                $0.exists && prefixes.contains(where: $0.label.hasPrefix)
            }) {
                hidden.swipeUp()
            }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return nil
    }


    func text(_ labels: [String], timeout: TimeInterval = 30) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for label in labels {
                let element = app.staticTexts.matching(NSPredicate(format: "label == %@", label)).firstMatch
                if element.exists { return element }
            }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return nil
    }

    /// 某个文案在屏上的条数（语言变体取最多的那个）：用来数“等待对方同意”行有几条。
    func countText(_ labels: [String]) -> Int {
        labels.map { app.staticTexts.matching(NSPredicate(format: "label == %@", $0)).count }.max() ?? 0
    }

    func containsText(_ needle: String, timeout: TimeInterval = 10) -> XCUIElement? {
        let predicate = NSPredicate(format: "label CONTAINS %@", needle)
        let element = app.staticTexts.matching(predicate).firstMatch
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if element.exists { return element }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return nil
    }

    func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "\(prefix)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// 引导向导（步骤 N／M）还在：侧边栏按钮在向导背后已经存在，光看它会把向导当主界面。
    func wizardUp() -> Bool {
        let steps = NSPredicate(format: "label BEGINSWITH %@ OR label BEGINSWITH %@ OR label BEGINSWITH %@ OR label BEGINSWITH %@",
                                "ステップ", "Step", "步骤", "步驟")
        return app.staticTexts.matching(steps).firstMatch.exists
    }

    /// 引导第一步的姓名是必填，空着“下一步”是灰的。
    func fillProfileName() {
        let names = NSPredicate(format: "label IN %@", ["Name", "名前", "名字", "名稱"])
        let field = app.textFields.matching(names).firstMatch
        guard field.exists, field.isHittable else { return }
        guard ((field.value as? String) ?? "").isEmpty else { return }
        field.tap()
        field.typeText("689 Walkthrough")
    }

    /// 按时间等，不按轮数：栈忙时启动配对就要几十秒，固定 90 轮（每轮 0.5–1 秒）会在配对中途用完。
    func passOnboarding() {
        let deadline = Date().addingTimeInterval(150)
        while Date() < deadline {
            if UITestBackend.skipKeyHandoff(app) { continue }
            // 向导出现的当口，标题那行会比栏晚一帧进无障碍树：侧边栏在且隔一拍仍没有步骤号才算到了主界面。
            if UITestSidebar.waitForMain(app, timeout: 0.5) && !wizardUp() {
                Thread.sleep(forTimeInterval: 1)
                if !wizardUp() { return }
            }
            fillProfileName()
            if let button = any(["Go to Chat", "チャットへ", "Done", "完了", "Next", "次へ", "Skip", "スキップ"], timeout: 1),
               button.isEnabled {
                button.tap()
                continue
            }
            let cell = app.cells.firstMatch
            if cell.exists && cell.isHittable { cell.tap() }
        }
    }

    func enterFriends() {
        passOnboarding()
        UITestBackend.settleKeyHandoff(app)
        if !UITestSidebar.waitForMain(app, timeout: 15) {
            // 引导的点击可能顺手拉开了侧边栏，开着时栏按钮不在：点右侧空白收起再等。
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
            _ = UITestSidebar.waitForMain(app, timeout: 10)
        }
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 5), "没进主界面")
        openFriends()
    }

    func openFriends() {
        // 设置行在抽屉最下面，可能露不出来：anyPrefix 会贴着它往下滚到能点。
        let open = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(open.waitForExistence(timeout: 15), "没有侧边栏按钮")
        open.tap()
        guard let settings = anyPrefix(L.settings, timeout: 30) else {
            return XCTFail("没有 Settings")
        }
        settings.tap()
        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 15), "设置没打开")
        guard let friends = anyPrefix(L.friends, timeout: 90) else {
            return XCTFail("设置里没有好友入口")
        }
        friends.tap()
        XCTAssertNotNil(any(L.add, timeout: 30), "好友页没打开")
    }

    @discardableResult
    func openAdd() -> XCUIElement {
        guard let add = any(L.add, timeout: 30) else {
            XCTFail("没有加好友按钮")
            return app.textFields.firstMatch
        }
        add.tap()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 15), "没有输入框")
        return field
    }

    /// 引导完整、占位不被压缩到只剩一行、placeholder 是全句而不是被截断的前缀。
    func assertEntry() {
        let field = app.textFields.firstMatch
        var help: XCUIElement!
        for expected in L.help {
            let element = app.staticTexts.matching(NSPredicate(format: "label == %@", expected)).firstMatch
            if element.waitForExistence(timeout: 5) { help = element; break }
        }
        XCTAssertNotNil(help, "没看到取址引导")
        guard let help else { return }
        XCTAssertGreaterThan(help.frame.height, 28, "引导只有一行，示例可能被挤掉")
        XCTAssertGreaterThan(field.frame.width, 150, "输入框太窄")
        let placeholder = field.placeholderValue ?? ""
        XCTAssertTrue(L.placeholder.contains(placeholder), "占位文字是 \(placeholder.debugDescription)")
    }
    /// 关掉当前的 sheet：先收键盘，再拉下 sheet。
    func closeSheet() {
        // 系统 sheet 在 iOS 26 上靠上沿往下拖才收：从抓手上拉，不从内容里滑。
        let grab = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.12))
        let bottom = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85))
        grab.press(forDuration: 0.05, thenDragTo: bottom)
        Thread.sleep(forTimeInterval: 0.8)
        if app.textFields.firstMatch.exists {
            grab.press(forDuration: 0.05, thenDragTo: bottom)
        }
    }

    /// 本人链接：从“我的二维码”读出的真实 matrix.to 链接。用来取本服域名，也用来加自己。
    func ownLink() -> String {
        _ = openAdd()
        guard let code = any(L.myCode, timeout: 30) else {
            XCTFail("没有本人码入口")
            return ""
        }
        code.tap()
        let link = app.staticTexts["friends-my-link"]
        XCTAssertTrue(link.waitForExistence(timeout: 30), "没有本人链接")
        let value = link.label
        closeSheet()
        XCTAssertTrue(link.waitForNonExistence(timeout: 10), "本人码没关掉")
        return value
    }

    /// 本人链接里的本服域名。
    func server(of link: String) -> String {
        guard let tail = link.split(separator: ":").last else { return "" }
        return String(tail)
    }

    /// 屏幕当前是不是深色：把整屏缩到 1 个 RGBA 像素读亮度。截图原来的像素布局带 alpha，直接读前三个
    /// 字节会把透明度当颜色算，暗色也会被判成亮，深色就一直切不过去。
    /// 取整屏缩到 1 个 RGBA 像素的亮度和四个字节，失败时写进日志便于定位。
    func screenLuma() -> (Double, [UInt8]) {
        guard let shot = XCUIScreen.main.screenshot().image.cgImage else { return (-1, []) }
        var pixel = [UInt8](repeating: 0, count: 4)
        let space = CGColorSpaceCreateDeviceRGB()
        let info = CGImageAlphaInfo.premultipliedLast.rawValue
        guard let context = CGContext(
            data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
            space: space, bitmapInfo: info
        ) else { return (-1, []) }
        context.draw(shot, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        let luma = 0.299 * Double(pixel[0]) + 0.587 * Double(pixel[1]) + 0.114 * Double(pixel[2])
        return (luma, pixel)
    }

    func screenIsDark() -> Bool { screenLuma().0 < 128 }

    /// 深色靠 App 的截图钩子（WONTON_SHOT_SCHEME）在启动时钉住：换下一个登录码、空数据目录重开，再回到同一页。
    func relaunchDark() {
        app.launchEnvironment["WONTON_SHOT_SCHEME"] = "dark"
        app.terminate()
        do { try UITestBackend.passPairingCode(to: app) } catch { XCTFail("换码失败：\(error)") }
        UITestBackend.launch(app)
        let deadline = Date().addingTimeInterval(40)
        while Date() < deadline {
            if screenIsDark() { return }
            Thread.sleep(forTimeInterval: 1)
        }
        XCTFail("重开后屏幕不是深色（luma=\(screenLuma().0)）")
    }

    /// 系统外观落不到 App（App 有自己的主题设置），深色从设置里换：好友页 → 设置 → 外観 → ダーク。
    func setThemeDark() {
        if app.textFields.firstMatch.exists { closeSheet() }
        guard let back = any(L.back, timeout: 20) else { return XCTFail("好友页没有返回") }
        back.tap()
        guard let appearance = any(L.appearance, timeout: 30) else { return XCTFail("设置里没有外观") }
        appearance.tap()
        guard let dark = any(L.dark, timeout: 30) else { return XCTFail("没有深色选项") }
        dark.tap()
        let deadline = Date().addingTimeInterval(20)
        while Date() < deadline {
            if screenIsDark() { return }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTFail("主题没切成深色（luma=\(screenLuma().0)）")
    }

    /// 从外观页回设置、再进好友页。
    func backToFriends() {
        guard let back = any(L.back, timeout: 20) else { return XCTFail("外观页没有返回") }
        back.tap()
        guard let friends = anyPrefix(L.friends, timeout: 60) else { return XCTFail("设置里没有好友入口") }
        friends.tap()
        XCTAssertNotNil(any(L.add, timeout: 30), "好友页没打开")
    }

    /// 外观设置默认跟随系统：换系统外观即可，页面在栈上也能就地变。切完等到真的变过去，截图才是深色。
    func setAppearance(dark: Bool) {
        XCUIDevice.shared.appearance = dark ? .dark : .light
        // 系统切外观要几秒才落到画面，截图也会先给几帧旧的：给 25 秒，每秒看一次。
        let deadline = Date().addingTimeInterval(25)
        while Date() < deadline {
            if screenIsDark() == dark { return }
            Thread.sleep(forTimeInterval: 1)
        }
        let last = screenLuma()
        XCTFail("\(dark ? "外观没切成深色" : "外观没切回浅色")（luma=\(last.0) bytes=\(last.1)）")
    }
}

final class Friend689EntryJATests: Friend689Base {
    override var japanese: Bool { true }

    func testLightAndDarkEntry() {
        enterFriends()
        _ = openAdd()
        assertEntry()
        shot("entry-light")
        setThemeDark()
        backToFriends()
        _ = openAdd()
        assertEntry()
        shot("entry-dark")
    }
}

final class Friend689EntryUITests: Friend689Base {
    func testLightAndDarkEntry() {
        enterFriends()
        _ = openAdd()
        shot("entry-light")
        setThemeDark()
        backToFriends()
        _ = openAdd()
        assertEntry()
        shot("entry-dark")
    }
}

final class Friend689ErrorUITests: Friend689Base {
    func testErrors() {
        enterFriends()
        let own = ownLink()
        XCTAssertFalse(own.isEmpty, "没取到本人链接")
        let server = server(of: own)
        XCTAssertFalse(server.isEmpty, "本人链接里没有服务器")

        // 本地非法：明确说不是 Matrix ID，并要真实地址。
        let field = openAdd()
        field.tap()
        field.typeText("bob")
        guard let sendButton = any(L.send, timeout: 10) else { return XCTFail("没有发送按钮") }
        sendButton.tap()
        XCTAssertNotNil(text(L.invalid, timeout: 30), "没有本地非法提示")
        shot("error-invalid-light")

        // 合法格式但查不到资料：中性未知，不说账号不存在/地址非法。
        let unknown = openAdd()
        unknown.tap()
        unknown.typeText("@nobody689:\(server)")
        guard let unknownSend = any(L.send, timeout: 10) else { return XCTFail("没有发送按钮") }
        unknownSend.tap()
        XCTAssertNotNil(text(L.upstream, timeout: 90), "没有中性的未知错误")
        XCTAssertNil(text(L.invalid, timeout: 1), "未知账号被说成地址非法")
        XCTAssertNil(text(L.forbidden, timeout: 1), "未知账号被说成请求禁止")
        shot("error-upstream-light")

        // 加自己：请求未获允许，详情有 403 和 errcode。
        let selfField = openAdd()
        selfField.tap()
        selfField.typeText(own)
        guard let ownSend = any(L.send, timeout: 10) else { return XCTFail("没有发送按钮") }
        ownSend.tap()
        XCTAssertNotNil(text(L.forbidden, timeout: 90), "没有禁止提示")
        XCTAssertNil(text(L.invalid, timeout: 1), "禁止被说成地址非法")
        shot("error-forbidden-light")
        if let details = any(L.details, timeout: 10) {
            details.tap()
            XCTAssertNotNil(containsText("403", timeout: 10), "详情里没有 403")
            shot("error-forbidden-details")
        }

        setThemeDark()
        backToFriends()
        let ownDark = ownLink()
        let fieldDark = openAdd()
        fieldDark.tap()
        fieldDark.typeText(ownDark)
        guard let sendDark = any(L.send, timeout: 10) else { return XCTFail("没有发送按钮") }
        sendDark.tap()
        XCTAssertNotNil(text(L.forbidden, timeout: 90), "深色下禁止提示没了")
        shot("error-forbidden-dark")
        XCTAssertNotNil(any(L.retry, timeout: 10), "没有重试按钮")
    }
}

final class Friend689SuccessUITests: Friend689Base {
    func testAddsAnOrdinaryContact() {
        enterFriends()
        let server = server(of: ownLink())
        XCTAssertFalse(server.isEmpty, "本人链接里没有服务器")
        let field = openAdd()
        field.tap()
        field.typeText("\(L.contactUser)\(server)")
        guard let sendButton = any(L.send, timeout: 10) else { return XCTFail("没有发送按钮") }
        let before = countText(L.waiting)
        sendButton.tap()
        // 夹具看到邀请 25 秒后代对方同意。以“等待行条数”为节拍：发出去应当多一条，被接受后回落到发之前。
        let sentDeadline = Date().addingTimeInterval(120)
        var appeared = false
        while Date() < sentDeadline {
            if countText(L.waiting) > before { appeared = true; break }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTAssertTrue(appeared, "邀请没发出去")
        XCTAssertNotNil(text([L.contactName], timeout: 30), "没有对方请求行")
        shot("add-success-light")
        // 对方接受：好友行出现。旧的“等待对方同意”行可能一直留着，不以它消失为准。
        XCTAssertNotNil(text(L.noGroups, timeout: 180), "对方没接受")
        XCTAssertNotNil(text([L.contactName], timeout: 30), "没有好友行")
        shot("add-success-accepted-light")
        setThemeDark()
        backToFriends()
        XCTAssertNotNil(text([L.contactName], timeout: 60), "深色下好友行没了")
        XCTAssertNotNil(text(L.noGroups, timeout: 30), "深色下没有好友行")
        shot("add-success-dark")
    }
}
