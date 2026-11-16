import XCTest
@testable import Wonton

/// 重新加载回哪个地址：正在看的那一页；登录往返里一次性的地址不重放，回入口。
final class TabReloadTargetTests: XCTestCase {
    private let entry = URL(string: "http://photos.home.example/")!
    private let landing = URL(string: "http://photos.home.example/auth/login?code=abc&state=xyz")!
    private let login = TabLoginRoute(domain: "home.example")

    private func target(loginLanding: URL? = nil) -> TabReloadTarget {
        TabReloadTarget(entry: entry, login: login, loginLanding: loginLanding)
    }

    func testReloadStaysOnTheCurrentPage() {
        let album = URL(string: "http://photos.home.example/albums/42")!
        XCTAssertEqual(target().resolve(recovery: nil, current: album), album)
        XCTAssertEqual(target(loginLanding: landing).resolve(recovery: nil, current: album), album)
    }

    func testRecoveryAddressWinsOverTheCurrentOne() {
        let failed = URL(string: "http://photos.home.example/search?q=cat")!
        let shown = URL(string: "http://photos.home.example/albums/42")!
        XCTAssertEqual(target().resolve(recovery: failed, current: shown), failed)
    }

    func testNothingLoadedYetGoesToTheEntry() {
        XCTAssertEqual(target().resolve(recovery: nil, current: nil), entry)
    }

    func testLoginLandingIsNotReplayed() {
        XCTAssertEqual(target(loginLanding: landing).resolve(recovery: nil, current: landing), entry)
        XCTAssertEqual(target(loginLanding: landing).resolve(recovery: landing, current: nil), entry)
    }

    func testAddressesOnTheLoginRouteAreNotReplayed() {
        let auth = URL(string: "http://auth.home.example/oauth2/auth?login_verifier=v")!
        let core = URL(string: "http://home.example/_wonton/oidc/login?login_challenge=c")!
        XCTAssertEqual(target().resolve(recovery: nil, current: auth), entry)
        XCTAssertEqual(target().resolve(recovery: core, current: nil), entry)
    }

    func testLoginRouteCoversOnlyTheAuthHostAndTheCoreOidcPath() {
        XCTAssertTrue(login.contains(URL(string: "http://auth.home.example/oauth2/auth")!))
        XCTAssertTrue(login.contains(URL(string: "http://home.example/_wonton/oidc/consent")!))
        XCTAssertFalse(login.contains(URL(string: "http://home.example/_wonton/api/me")!))
        XCTAssertFalse(login.contains(URL(string: "http://photos.home.example/auth/login")!))
        XCTAssertFalse(TabLoginRoute(domain: nil).contains(URL(string: "http://auth.home.example/")!))
    }
}
