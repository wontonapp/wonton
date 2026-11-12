import XCTest

/// 用 Matrix 账号登录，连真实的 matrix.org：OIDC 打开 matrix.org 的登录页，取消后回到起手；从网页登录面板以外送来
/// 一个对不上的回调，核心判它对不上。不建账号、不输密码。每跑一次都在 matrix.org 动态注册一个客户端，所以夜跑不跑，
/// 手动跑时设 `TEST_RUNNER_MATRIX_LOGIN_LIVE=1`。
final class MatrixLoginUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["MATRIX_LOGIN_LIVE"] == nil,
            "要连真实的 matrix.org：设 TEST_RUNNER_MATRIX_LOGIN_LIVE=1"
        )
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        // 直连不了 matrix.org 的跑测机器经代理出去（`TEST_RUNNER_MATRIX_LOGIN_PROXY=http://…`）；核心的 HTTP 客户端认这个变量。
        if let proxy = ProcessInfo.processInfo.environment["MATRIX_LOGIN_PROXY"] {
            app.launchEnvironment["HTTPS_PROXY"] = proxy
        }
    }

    func testOidcOpensMatrixOrgAndForeignCallbackFails() throws {
        app.launch()
        openMatrixLogin()
        XCTAssertEqual(app.textFields["matrix-login-server"].value as? String, "matrix.org")

        startOidc()
        shot("01-matrix-org-sign-in")
        // 系统面板左上角的关闭：登录取消，回到起手。按钮文字跟系统语言走。
        let labels = ["Cancel", "Close", "取消", "关闭"]
        let cancel = app.buttons.matching(NSPredicate(format: "label IN %@", labels)).firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 10), "网页登录面板没有关闭按钮\n\(app.debugDescription)")
        cancel.tap()
        let start = app.buttons["matrix-login-start"]
        XCTAssertTrue(start.waitForExistence(timeout: 10), "取消后没回到起手\n\(app.debugDescription)")
        XCTAssertEqual(start.label, "Continue")
        shot("02-cancelled")

        startOidc()
        // 面板以外送来的回调（state 是假的）：App 交给核心，核心判它对不上，这次登录失败。
        XCUIDevice.shared.system.open(try XCTUnwrap(URL(string: "io.github.wontonapp:/matrix-login?code=x&state=y")))
        // 系统可能先问一句“在 App 中打开？”，右边是打开。
        if let prompt = systemAlert(timeout: 5) { prompt.buttons.element(boundBy: 1).tap() }
        let failure = app.staticTexts["The sign-in result from the browser doesn’t match"]
        XCTAssertTrue(failure.waitForExistence(timeout: 10), "对不上的回调没让登录失败\n\(app.debugDescription)")
        XCTAssertEqual(start.label, "Try Again")
        shot("03-foreign-callback")
    }

    /// 全新安装落在配对页，“用 Matrix 账号登录”就在配对表单下面。
    private func openMatrixLogin() {
        let entry = app.buttons["Sign In with a Matrix Account"].firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 20), "配对页没有 Matrix 登录\n\(app.debugDescription)")
        entry.tap()
        XCTAssertTrue(app.textFields["matrix-login-server"].waitForExistence(timeout: 10), "登录页没打开")
    }

    /// 发现 matrix.org 支持的方式，选浏览器登录，过系统的“要用 matrix.org 登录”确认，等 matrix.org 的页面出来。
    private func startOidc() {
        app.buttons["matrix-login-start"].tap()
        let oidc = app.buttons["matrix-login-oidc"]
        XCTAssertTrue(oidc.waitForExistence(timeout: 90), "matrix.org 没给出浏览器登录\n\(app.debugDescription)")
        oidc.tap()
        // 系统确认“要用 matrix.org 登录”，文字跟系统语言走，右边是继续。
        let allow = systemAlert(timeout: 90)
        XCTAssertNotNil(allow, "没弹出系统的登录确认\n\(app.debugDescription)")
        allow?.buttons.element(boundBy: 1).tap()
        let page = app.webViews.firstMatch
        XCTAssertTrue(page.waitForExistence(timeout: 90), "matrix.org 的登录页没打开\n\(app.debugDescription)")
    }

    /// 系统弹的确认框，可能挂在 SpringBoard 上，也可能挂在 App 里。
    private func systemAlert(timeout: TimeInterval) -> XCUIElement? {
        let candidates = [
            XCUIApplication(bundleIdentifier: "com.apple.springboard").alerts.firstMatch,
            app.alerts.firstMatch
        ]
        let shown = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in candidates.contains { $0.exists } }, object: nil
        )
        guard XCTWaiter().wait(for: [shown], timeout: timeout) == .completed else { return nil }
        return candidates.first { $0.exists }
    }

    private func shot(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
