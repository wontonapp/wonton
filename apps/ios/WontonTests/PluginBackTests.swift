import XCTest
@testable import Wonton

final class PluginBackTests: XCTestCase {
    func testBackTargetPrefersClosingCustomViewThenOnlyGoesBackForCrossDocumentSteps() {
        XCTAssertEqual(pluginBackTarget(customView: true, history: nil), .closeCustomView)
        XCTAssertEqual(
            pluginBackTarget(customView: true, history: HistoryStep(sameDocument: false, previous: "/old")),
            .closeCustomView
        )
        XCTAssertEqual(pluginBackTarget(customView: false, history: nil), .closePage)
        XCTAssertEqual(
            pluginBackTarget(customView: false, history: HistoryStep(sameDocument: true, previous: "/same")),
            .closePage
        )
        XCTAssertEqual(
            pluginBackTarget(customView: false, history: HistoryStep(sameDocument: false, previous: "/old")),
            .history
        )
    }

    func testLogUrlDropsQueryAndFragment() {
        XCTAssertEqual(logUrl("http://app.test/app/?code=abc#/album/2"), "http://app.test/app/")
        XCTAssertEqual(logUrl("http://app.test/app/#/album/2"), "http://app.test/app/")
        XCTAssertEqual(logUrl("http://app.test/app/"), "http://app.test/app/")
        XCTAssertEqual(logUrl("http://app.test/app/?"), "http://app.test/app/")
    }

    func testSameUrlIsSameDocumentButFragmentOnlyStepIsNot() {
        let history = PageHistory()
        history.visited("http://app.test/app/")
        XCTAssertTrue(history.isSameDocument("http://app.test/app/"))
        XCTAssertFalse(history.isSameDocument("http://app.test/app/#/album/2"))
        XCTAssertFalse(history.isSameDocument("http://app.test/other/"))
    }

    func testRedirectSourceCountsAsSameDocument() {
        let history = PageHistory()
        history.started("http://app.test/")
        history.visited("http://app.test/app/")
        history.finished("http://app.test/app/")
        XCTAssertTrue(history.isSameDocument("http://app.test/"))
        XCTAssertFalse(history.isSameDocument("http://app.test/old/"))
    }

    func testPagePathDropsQueryAndFragment() {
        let history = PageHistory()
        XCTAssertNil(history.pagePath())
        history.visited("http://app.test/app/?code=abc#/album/2")
        XCTAssertEqual(history.pagePath(), "http://app.test/app/")
    }

    func testClearForgetsCurrentUrlAndRedirects() {
        let history = PageHistory()
        history.started("http://app.test/")
        history.finished("http://app.test/app/")
        history.clear()
        XCTAssertNil(history.pagePath())
        XCTAssertFalse(history.isSameDocument("http://app.test/"))
    }
}
