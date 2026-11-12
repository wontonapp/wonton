import XCTest

/// 用 Matrix 账号登录，对着本地桩（`MatrixLoginStub`）走取消、回调对不上、重试。不连外网，夜跑和日常都跑。
final class MatrixLoginUITests: MatrixLoginFlow {
    private var stub: MatrixLoginStub?

    override func setUpWithError() throws {
        try super.setUpWithError()
        let stub = try MatrixLoginStub()
        try stub.start()
        self.stub = stub
        // 跑测机器的系统代理不能接管发往本机桩的请求。
        app.launchEnvironment["NO_PROXY"] = "127.0.0.1,localhost"
    }

    override func tearDown() {
        stub?.stop()
        super.tearDown()
    }

    func testCancelMismatchedCallbackAndRetry() throws {
        let stub = try XCTUnwrap(stub)
        app.launch()
        openMatrixLogin()
        let server = app.textFields["matrix-login-server"]
        // 点在右端，光标落在 matrix.org 后面，再删干净换成桩的地址。
        server.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.5)).tap()
        server.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 20) + stub.url)

        startOidc(timeout: 10)
        closeSheet()
        let start = app.buttons["matrix-login-start"]
        XCTAssertTrue(start.waitForExistence(timeout: 10), "取消后没回到起手\n\(app.debugDescription)")
        XCTAssertEqual(start.label, "Continue")
        shot("01-cancelled")

        // 授权页交回 state 对不上的回调：核心判它对不上，这次登录失败，可以重试。
        startOidc(timeout: 10)
        let mismatch = app.links[MatrixLoginStub.mismatchLink].firstMatch
        XCTAssertTrue(mismatch.waitForExistence(timeout: 10), "桩的授权页没打开\n\(app.debugDescription)")
        mismatch.tap()
        XCTAssertTrue(failureText.waitForExistence(timeout: 10), "对不上的回调没让登录失败\n\(app.debugDescription)")
        XCTAssertEqual(start.label, "Try Again")
        shot("02-mismatched-callback")

        start.tap()
        XCTAssertTrue(
            app.buttons["matrix-login-oidc"].waitForExistence(timeout: 10),
            "重试没重新走到选方式\n\(app.debugDescription)"
        )
        shot("03-retried")
    }
}

/// 连真实的 matrix.org：打开它的登录页，取消后回到起手；从网页登录面板以外送来对不上的回调，核心判它对不上。
/// 不建账号、不输密码。默认跳过：要连外网，每跑一次都在 matrix.org 动态注册一个客户端，跑测机器连不上 matrix.org 时
/// 只会失败在网络上。手动运行（直连不了时加代理）：
///
///     TEST_RUNNER_MATRIX_LOGIN_LIVE=1 TEST_RUNNER_MATRIX_LOGIN_PROXY=http://127.0.0.1:7890 \
///       xcodebuild test -project apps/ios/Wonton.xcodeproj -scheme Wonton \
///       -destination 'platform=iOS Simulator,name=<模拟器>' \
///       -only-testing:WontonUITests/MatrixLoginLiveUITests CODE_SIGNING_ALLOWED=NO
final class MatrixLoginLiveUITests: MatrixLoginFlow {
    override func setUpWithError() throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["MATRIX_LOGIN_LIVE"] == nil,
            "连真实的 matrix.org，默认跳过；手动运行设 TEST_RUNNER_MATRIX_LOGIN_LIVE=1"
        )
        try super.setUpWithError()
        // 核心的 HTTP 客户端认这个变量。
        if let proxy = ProcessInfo.processInfo.environment["MATRIX_LOGIN_PROXY"] {
            app.launchEnvironment["HTTPS_PROXY"] = proxy
        }
    }

    func testOidcOpensMatrixOrgAndForeignCallbackFails() throws {
        app.launch()
        openMatrixLogin()
        XCTAssertEqual(app.textFields["matrix-login-server"].value as? String, "matrix.org")

        startOidc(timeout: 90)
        shot("01-matrix-org-sign-in")
        closeSheet()
        let start = app.buttons["matrix-login-start"]
        XCTAssertTrue(start.waitForExistence(timeout: 10), "取消后没回到起手\n\(app.debugDescription)")
        XCTAssertEqual(start.label, "Continue")
        shot("02-cancelled")

        startOidc(timeout: 90)
        // 面板以外送来的回调（state 是假的）：App 交给核心，核心判它对不上。
        XCUIDevice.shared.system.open(try XCTUnwrap(URL(string: "io.github.wontonapp:/matrix-login?code=x&state=y")))
        // 系统可能先问一句“在 App 中打开？”，右边是打开。
        if let prompt = systemAlert(timeout: 5) { prompt.buttons.element(boundBy: 1).tap() }
        XCTAssertTrue(failureText.waitForExistence(timeout: 10), "对不上的回调没让登录失败\n\(app.debugDescription)")
        XCTAssertEqual(start.label, "Try Again")
        shot("03-foreign-callback")
    }
}

/// 两组用例共用的步骤。
class MatrixLoginFlow: XCTestCase {
    var app: XCUIApplication!

    var failureText: XCUIElement { app.staticTexts["The sign-in result from the browser doesn’t match"] }

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
    }

    /// 全新安装落在配对页，“用 Matrix 账号登录”就在配对表单下面。
    func openMatrixLogin() {
        let entry = app.buttons["Sign In with a Matrix Account"].firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 20), "配对页没有 Matrix 登录\n\(app.debugDescription)")
        entry.tap()
        XCTAssertTrue(app.textFields["matrix-login-server"].waitForExistence(timeout: 10), "登录页没打开")
    }

    /// 发现服务器支持的方式，选浏览器登录，过系统的“要用某某登录”确认，等授权页出来。`timeout` 是等服务器的时间。
    func startOidc(timeout: TimeInterval) {
        app.buttons["matrix-login-start"].tap()
        let oidc = app.buttons["matrix-login-oidc"]
        XCTAssertTrue(oidc.waitForExistence(timeout: timeout), "服务器没给出浏览器登录\n\(app.debugDescription)")
        oidc.tap()
        // 系统的确认，文字跟系统语言走，右边是继续。
        let allow = systemAlert(timeout: timeout)
        XCTAssertNotNil(allow, "没弹出系统的登录确认\n\(app.debugDescription)")
        allow?.buttons.element(boundBy: 1).tap()
        let page = app.webViews.firstMatch
        XCTAssertTrue(page.waitForExistence(timeout: timeout), "授权页没打开\n\(app.debugDescription)")
    }

    /// 网页登录面板左上角的关闭，文字跟系统语言走。
    func closeSheet() {
        let labels = ["Cancel", "Close", "取消", "关闭"]
        let close = app.buttons.matching(NSPredicate(format: "label IN %@", labels)).firstMatch
        XCTAssertTrue(close.waitForExistence(timeout: 10), "网页登录面板没有关闭按钮\n\(app.debugDescription)")
        close.tap()
    }

    /// 系统弹的确认框，可能挂在 SpringBoard 上，也可能挂在 App 里。
    func systemAlert(timeout: TimeInterval) -> XCUIElement? {
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

    func shot(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
