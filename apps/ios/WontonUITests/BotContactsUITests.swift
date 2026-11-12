import XCTest

/// Bot 的联系人页（ADR-0045、ADR-0050）走屏：一个 Bot 请另一个 Bot 做联系人，另一个 Bot 的联系人页里出现请求，
/// 同意后两边都是“已添加”。连开发栈：两个 Bot 归同一个主人，请求和同意都在这台 App 上做。
///
/// 开发栈事先装好两个回声 Bot（第二个是同一个包换了 ID 和名字），名字和第一个 Bot 的用户 ID 经环境变量传：
/// `CONTACTS_BOT_A`（被请求的那个，默认 Echo）、`CONTACTS_BOT_B`（发请求的那个）、`CONTACTS_USER_A`（A 的完整用户 ID）。
/// 每一步都留截图附件。
final class BotContactsUITests: XCTestCase {
    private var app: XCUIApplication!
    /// 界面上的元素：最多等 10 秒。
    private let uiWait: TimeInterval = 10
    /// 要经过家里的服务器的：最多等 90 秒。
    private let networkWait: TimeInterval = 90

    private var env: [String: String] { ProcessInfo.processInfo.environment }

    override func setUpWithError() throws {
        continueAfterFailure = false
        try UITestBackend.require()
        try XCTSkipIf(env["CONTACTS_BOT_B"] == nil || env["CONTACTS_USER_A"] == nil,
                      "需要 CONTACTS_BOT_B 和 CONTACTS_USER_A")
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        UITestBackend.passPairingCode(to: app)
        app.launch()
    }

    func testRequestThenApproveBetweenTwoBots() throws {
        let botA = env["CONTACTS_BOT_A"] ?? "Echo"
        let botB = try XCTUnwrap(env["CONTACTS_BOT_B"])
        let userA = try XCTUnwrap(env["CONTACTS_USER_A"])
        reachApps()

        // B 请 A 做联系人：B 这边是“等对方同意”，可撤销。
        openContacts(of: botB)
        shot("01-b-contacts")
        app.buttons["Add contact"].firstMatch.tap()
        let field = app.textFields["contactsUserId"]
        XCTAssertTrue(field.waitForExistence(timeout: uiWait), "加联系人的面板没出来")
        field.tap()
        field.typeText(userA)
        shot("02-add-sheet")
        app.buttons["contactsSubmit"].tap()
        XCTAssertTrue(app.buttons["Withdraw"].waitForExistence(timeout: networkWait), "B 这边没有等对方的行")
        shot("03-b-pending")

        // A 的联系人页里有这条请求，同意后进“已添加”。
        goBack()
        goBack()
        openContacts(of: botA)
        let approve = app.buttons["Approve"].firstMatch
        XCTAssertTrue(approve.waitForExistence(timeout: networkWait), "A 的联系人页没有收到请求")
        shot("04-a-incoming")
        approve.tap()
        XCTAssertTrue(app.buttons["Remove contact"].waitForExistence(timeout: networkWait), "同意后 A 这边没有生效")
        XCTAssertFalse(app.buttons["Approve"].exists, "同意后请求还在")
        shot("05-a-active")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// 冷启动连上家里的服务器，走完或跳过引导，经侧栏“我的应用 · 全部”到应用页。
    private func reachApps() {
        let sidebar = app.buttons["Sidebar"].firstMatch
        for _ in 0..<8 where !sidebar.waitForExistence(timeout: uiWait) {
            let step = ["Skip", "Done", "Next"].map { app.buttons[$0].firstMatch }.first { $0.exists && $0.isEnabled }
            step?.tap()
        }
        XCTAssertTrue(sidebar.waitForExistence(timeout: networkWait), "没到主界面")
        sidebar.tap()
        let all = app.buttons["All"].firstMatch
        XCTAssertTrue(all.waitForExistence(timeout: uiWait), "侧栏没有“全部”")
        all.tap()
    }

    /// 应用页长按 Bot 的图标进详情，再进联系人页。
    private func openContacts(of bot: String) {
        let tile = app.buttons.containing(.staticText, identifier: bot).firstMatch
        XCTAssertTrue(tile.waitForExistence(timeout: networkWait), "应用页没有 \(bot)")
        tile.press(forDuration: 1.5)
        let details = app.buttons["Details"].firstMatch
        XCTAssertTrue(details.waitForExistence(timeout: uiWait), "长按菜单没有详情")
        details.tap()
        let entry = app.buttons["botContactsEntry"]
        XCTAssertTrue(entry.waitForExistence(timeout: networkWait), "\(bot) 的详情页没有联系人入口")
        entry.tap()
        XCTAssertTrue(app.buttons["Add contact"].firstMatch.waitForExistence(timeout: uiWait), "没进联系人页")
    }

    private func goBack() {
        let back = app.buttons["Back"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: uiWait), "没有返回按钮")
        back.tap()
    }
}
