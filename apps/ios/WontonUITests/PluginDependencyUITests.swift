import XCTest

/// 依赖的界面走查（ADR-0053）：确认页要列出一起装的提供方和它的权限、密钥；卸载提供方要被在用的
/// 应用拦下，并给出一同卸载或先停掉的选择。假数据里是 Autobangumi（要 torrent_client）和 qBittorrent。
final class PluginDependencyUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
        XCTAssertTrue(UITestSidebar.waitForMain(app, timeout: 20), "主界面没起来\n\(app.debugDescription)")
    }

    func testInstallPreviewListsTheProviderWithItsPermissions() {
        openStore()
        let link = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Autobangumi")).firstMatch
        for _ in 0..<10 where !link.exists {
            app.swipeUp()
        }
        XCTAssertTrue(link.waitForExistence(timeout: 10), "商店里没有 Autobangumi\n\(app.debugDescription)")
        link.tap()
        XCTAssertTrue(
            app.staticTexts["Follows the season and grabs new episodes"].firstMatch
                .waitForExistence(timeout: 10),
            "没打开详情页\n\(app.debugDescription)"
        )
        let install = app.buttons.matching(
            NSPredicate(format: "label IN %@", ["Install", "Get"])
        ).firstMatch
        XCTAssertTrue(install.waitForExistence(timeout: 10), "详情页没有获取按钮\n\(app.debugDescription)")
        install.tap()

        let capability = app.staticTexts["torrent_client"].firstMatch
        XCTAssertTrue(capability.waitForExistence(timeout: 10), "确认页没有缺的能力\n\(app.debugDescription)")
        let provider = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS[c] %@", "qBittorrent")
        ).firstMatch
        XCTAssertTrue(provider.waitForExistence(timeout: 10), "确认页没有一起装的提供方\n\(app.debugDescription)")
        XCTAssertTrue(
            app.staticTexts["Write to the downloads folder"].firstMatch.waitForExistence(timeout: 10),
            "确认页没有列出提供方的权限\n\(app.debugDescription)"
        )
        XCTAssertTrue(app.staticTexts["API key"].firstMatch.exists, "确认页没有列出提供方要填的密钥")

        XCTAssertTrue(
            app.staticTexts["indexer"].firstMatch.exists,
            "缺的可选能力没有列出来"
        )
        attach("install-provider")
    }

    func testProviderInstallListsWantingApps() {
        openStore()
        let target = app.staticTexts["qBittorrent"].firstMatch
        for _ in 0..<12 where !target.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(target.isHittable, "商店里没有能点的 qBittorrent\n\(app.debugDescription)")
        target.tap()
        XCTAssertTrue(
            app.staticTexts["The household's downloads"].firstMatch.waitForExistence(timeout: 10),
            "没打开详情页\n\(app.debugDescription)"
        )
        let install = app.buttons.matching(NSPredicate(format: "label IN %@", ["Install", "Get"])).firstMatch
        XCTAssertTrue(install.waitForExistence(timeout: 10), "详情页没有获取按钮\n\(app.debugDescription)")
        install.tap()
        XCTAssertTrue(
            app.staticTexts["These apps want it"].firstMatch.waitForExistence(timeout: 10),
            "确认页没有列出想用这个提供方的应用\n\(app.debugDescription)"
        )
        XCTAssertTrue(app.staticTexts["Audio Book"].firstMatch.exists, "想用的应用没列出来")
        attach("install-wanting")
    }

    func testUninstallProviderShowsDependents() {
        openApps()
        let icon = app.staticTexts["qBittorrent"].firstMatch
        XCTAssertTrue(icon.waitForExistence(timeout: 10), "应用页没有 qBittorrent\n\(app.debugDescription)")
        icon.tap()
        let uninstall = app.buttons["Uninstall"].firstMatch
        XCTAssertTrue(uninstall.waitForExistence(timeout: 10), "详情页没有卸载\n\(app.debugDescription)")
        while !uninstall.isHittable { app.swipeUp() }
        uninstall.tap()

        XCTAssertTrue(
            app.staticTexts["These apps still need qBittorrent"].firstMatch.waitForExistence(timeout: 10),
            "卸载没有被拦下\n\(app.debugDescription)"
        )
        XCTAssertTrue(app.staticTexts["Autobangumi"].firstMatch.exists, "拦截没有列出依赖方")
        XCTAssertTrue(app.buttons["Uninstall them too"].firstMatch.exists, "没有一起卸载的选择")
        XCTAssertTrue(app.buttons["Stop them first"].firstMatch.exists, "没有先停掉的选项")
        attach("uninstall-in-use")
    }

    func testDependencyProblemOnTheAppPage() {
        openApps()
        let icon = app.staticTexts["Autobangumi"].firstMatch
        XCTAssertTrue(icon.waitForExistence(timeout: 10), "应用页没有 Autobangumi\n\(app.debugDescription)")
        icon.tap()
        XCTAssertTrue(
            app.staticTexts["qBittorrent isn't running, so this app can't use it right now."]
                .firstMatch.waitForExistence(timeout: 10),
            "详情页没有依赖没在运行的提示\n\(app.debugDescription)"
        )
        XCTAssertTrue(app.buttons["Start qBittorrent"].firstMatch.exists, "提示没有启动的下一步")
        attach("dependency-problem")
    }

    private func openStore() {
        openApps()
        tap("Store")
        XCTAssertTrue(
            app.staticTexts["Photo Vault"].firstMatch.waitForExistence(timeout: 10),
            "商店没打开\n\(app.debugDescription)"
        )
    }

    private func openApps() {
        guard let all = UITestSidebar.allApps(app) else {
            return XCTFail("没有全部应用入口\n\(app.debugDescription)")
        }
        all.tap()
        XCTAssertTrue(app.staticTexts["qBittorrent"].waitForExistence(timeout: 10), "应用页没打开")
    }

    private func tap(_ label: String) {
        let button = app.buttons[label].firstMatch.exists
            ? app.buttons[label].firstMatch
            : app.staticTexts[label].firstMatch
        XCTAssertTrue(button.waitForExistence(timeout: 10), "找不到 \(label)\n\(app.debugDescription)")
        button.tap()
    }

    private func attach(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
