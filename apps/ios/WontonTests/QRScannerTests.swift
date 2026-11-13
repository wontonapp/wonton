import XCTest
@testable import Wonton

@MainActor
final class QRScannerTests: XCTestCase {
    func testSingleShotStopsBeforeDeliveryAndBlocksEveryLaterCode() {
        var events: [String] = []
        let scanner = QRScannerView(singleShot: true, onCode: { events.append($0) }, onUnavailable: { _ in })
        let coordinator = scanner.makeCoordinator()

        coordinator.deliver("first", stopScanning: { events.append("stop") })
        coordinator.deliver("first", stopScanning: { events.append("stop") })
        coordinator.deliver("different", stopScanning: { events.append("stop") })

        XCTAssertEqual(events, ["stop", "first"])
        XCTAssertTrue(coordinator.hasDeliveredCode)
        let reopened = scanner.makeCoordinator()
        reopened.deliver("first", stopScanning: { events.append("stop") })
        XCTAssertEqual(events, ["stop", "first", "stop", "first"])
    }

    func testSharedScannerDefaultsToRepeatedDeliveryWithoutStopping() {
        var codes: [String] = []
        let scanner = QRScannerView(onCode: { codes.append($0) }, onUnavailable: { _ in })
        let coordinator = scanner.makeCoordinator()

        coordinator.deliver("first", stopScanning: { XCTFail("Continuous scanning must not stop") })
        coordinator.deliver("first", stopScanning: { XCTFail("Continuous scanning must not stop") })
        coordinator.deliver("different", stopScanning: { XCTFail("Continuous scanning must not stop") })

        XCTAssertEqual(codes, ["first", "first", "different"])
        XCTAssertFalse(coordinator.hasDeliveredCode)
    }
}
