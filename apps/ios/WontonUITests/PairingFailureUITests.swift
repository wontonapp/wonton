import XCTest

/// 输一个连不上的配对码：提交后要留在配对页显示失败，能看到总耗时（秒）和诊断，码文本不外露。
/// 走新鲜路径（未配对 → 引导的配对表单），中英各一轮（…Zh 是中文轮）。
/// 不走「已配对 → 设置 → 重新配对」：那条路在已连接的设备上点重新配对会立即重配并关掉设置
/// 弹层（1117 走查三帧实测），空表单只有离线设备才到得了，测试环境造不出离线；而且弹层里的
/// 失败面渲染在弹层自己的推送页上，根本不经过 MainView.showsPairingPage。新鲜路径 adding 恒为
/// false，失败面留不留完全看 showsPairingPage 的 `.failed` 分支，是对这个修复最纯的检验。
@MainActor
final class PairingFailureUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
        executionTimeAllowance = 900
        try UITestBackend.require()
    }

    /// 让配对真正失败：iroh 会用节点 id 经中继/DNS 自发现真实地址，只改 addr 挡不住（1117 控制台探针实测
    /// 3.6s 就 paired）。把 id 换成 64 个 0（没有这个节点），地址换成 TEST-NET，核心按各段超时走到 Failed。
    /// 核心要求 core 的主机名等于 domain，domain 留着原值，core 换掉端口。
    private func dead(_ code: String) throws -> String {
        var components = try XCTUnwrap(URLComponents(string: code))
        let domain = try XCTUnwrap((components.queryItems ?? []).first { $0.name == "domain" }?.value)
        let dropped = ["relays", "addr", "core"]
        var items = (components.queryItems ?? []).filter { !dropped.contains($0.name) }.map {
            $0.name == "id" ? URLQueryItem(name: "id", value: String(repeating: "0", count: 64)) : $0
        }
        items += [
            URLQueryItem(name: "addr", value: "192.0.2.10:19082"),
            URLQueryItem(name: "core", value: "http://\(domain):19080/")
        ]
        components.queryItems = items
        let url = try XCTUnwrap(components.url).absoluteString
        XCTAssertTrue(url.contains("id=0000") && url.contains("secret="), url)
        return url
    }

    /// 引导页：跳过优先，跳过不了就下一步/完成，直到配对表单或主界面出现。
    private func passOnboarding(_ app: XCUIApplication) {
        for _ in 0..<24 {
            if UITestBackend.skipKeyHandoff(app) { continue }
            if app.descendants(matching: .any)["pairing-code"].firstMatch.exists { return }
            if app.buttons.matching(UITestSidebar.button).firstMatch.exists { return }
            if let skip = any(app, ["Skip"]), skip.isEnabled { skip.tap(); continue }
            if let finish = any(app, ["Go to Chat"]), finish.isEnabled { finish.tap(); continue }
            if let done = any(app, ["Done"]), done.isEnabled { done.tap(); continue }
            if let next = any(app, ["Next"]), next.isEnabled { next.tap(); continue }
        }
    }

    /// 把当时的树和截图打印/挂到结果里，失败时不用重跑就能看清停在哪个页面。
    private func dump(_ app: XCUIApplication, _ name: String) {
        print("=== TREE \(name) ===\n\(app.debugDescription)\n=== END \(name) ===")
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func any(_ app: XCUIApplication, _ labels: [String]) -> XCUIElement? {
        for label in labels {
            let element = app.buttons[label].firstMatch
            if element.exists { return element }
        }
        return nil
    }

    func testFreshUnreachableCodeKeepsFailurePage() throws {
        try runFresh(verifyZh: false)
    }

    func testFreshUnreachableCodeKeepsFailurePageZh() throws {
        try runFresh(verifyZh: true)
    }

    /// 不给启动码（DEBUG 启动会无条件 pairHome，把用例变成另一条路）；每次启动用全新数据目录，
    /// 保证从未配对、引导从头走。断言带双语候选，防文案与语言错配。
    private func runFresh(verifyZh: Bool) throws {
        let app = XCUIApplication()
        app.launchArguments += verifyZh
            ? ["-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]
            : ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launch()
        passOnboarding(app)
        let field = app.descendants(matching: .any)["pairing-code"].firstMatch
        // 失败面的判据用 scan-again：失败面源码里有 pairing-failure 标识，但运行时 a11y 树不暴露它
        // （1117 走查实测树里只有外层 pairing-progress），scan-again 按钮是失败面独有的。
        let failure = app.buttons["pairing-scan-again"].firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 60), app.debugDescription)
        field.tap()
        field.typeText(try dead(try XCTUnwrap(UITestBackend.codes().first)))
        app.buttons["pairing-submit"].tap()
        // 提交后立刻和 30 秒各抓一帧：失败面若中途消失，两帧能对上时间线。
        dump(app, "submitted-0s")
        _ = failure.waitForExistence(timeout: 30)
        dump(app, "submitted-30s")
        // 交换各段超时的预算是 8×30s+两次绑定（pairing.rs:504）；死节点 id 实测 ~30s 就失败。
        XCTAssertTrue(failure.waitForExistence(timeout: 330), app.debugDescription)
        // 失败面就在这一页：表单不再出现，也没被换走。
        XCTAssertFalse(field.exists)
        assertFailureContent(verifyZh: verifyZh)
    }

    /// 失败面可见后的内容断言：秒数在 Details 展开层，诊断直连地址在 Diagnostics 层，最后留档截图。
    private func assertFailureContent(verifyZh: Bool) {
        let app = XCUIApplication()
        let details = app.buttons.matching(NSPredicate(
            format: "label IN %@", ["Details", "详情"]
        )).firstMatch
        XCTAssertTrue(details.waitForExistence(timeout: 10), app.debugDescription)
        details.tap()
        let message = app.staticTexts.matching(NSPredicate(
            format: "label CONTAINS %@", verifyZh ? "总耗时" : "Total elapsed"
        )).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(message.label.contains(verifyZh ? "秒" : "seconds"), message.label)

        let diagnostics = app.buttons.matching(NSPredicate(
            format: "label IN %@", ["Diagnostics", "诊断信息"]
        )).firstMatch
        XCTAssertTrue(diagnostics.waitForExistence(timeout: 10), app.debugDescription)
        diagnostics.tap()
        let direct = app.staticTexts.matching(NSPredicate(
            format: "label CONTAINS %@", verifyZh ? "直连地址" : "Direct addresses"
        )).firstMatch
        XCTAssertTrue(direct.waitForExistence(timeout: 10), app.debugDescription)
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "pairing-failure-stays"
        shot.lifetime = .keepAlways
        add(shot)
    }
}
