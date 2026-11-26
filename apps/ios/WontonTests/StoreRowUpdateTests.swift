import XCTest

@testable import Wonton

/// 行尾给不给“更新”：上传的包看来源，预装插件看核心查到的报价；两样都没有时核心升不了。
final class StoreRowUpdateTests: XCTestCase {
    func testPreinstalledPluginWithAnOfferStillShowsUpdate() {
        XCTAssertTrue(
            storeRowShowsUpdate(installed: "1.0.0", version: "2.0.0", hasSource: false, hasOffer: true)
        )
    }

    func testUploadedPluginWithASourceShowsUpdate() {
        XCTAssertTrue(
            storeRowShowsUpdate(installed: "1.0.0", version: "2.0.0", hasSource: true, hasOffer: false)
        )
    }

    func testPreinstalledPluginWithoutAnOfferGetsNoUpdate() {
        XCTAssertFalse(
            storeRowShowsUpdate(installed: "1.0.0", version: "2.0.0", hasSource: false, hasOffer: false)
        )
    }

    func testSameVersionGetsNoUpdate() {
        XCTAssertFalse(
            storeRowShowsUpdate(installed: "2.0.0", version: "2.0.0", hasSource: true, hasOffer: true)
        )
    }
}
