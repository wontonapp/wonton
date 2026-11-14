import XCTest

/// Official fake-core layout evidence only; this does not authorize or exercise real hardware.
/// Run with -only-testing:WontonUITests/GrantedDeviceDetailUITests.
final class GrantedDeviceDetailUITests: XCTestCase {
    private var app: XCUIApplication!
    private static let serialPath = "/dev/serial/by-id/usb-ITead_Sonoff_Zigbee_3.0-if00-port0"
    private static let paths = [
        serialPath,
        "/dev/bus/usb",
        "/dev/disk/by-id/nvme-Samsung_SSD_990_PRO_1TB",
        "/dev/disk/by-id/ata-WDC_WD40EFRX-68N32N0",
        "/dev/example-hardware"
    ]

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

    func testGrantedHardwareDetailLayoutAndDisclosure() {
        openHomeHubDetail()
        reveal(row(0))
        shot("01-list-collapsed")
        for index in 0..<5 {
            let device = row(index)
            reveal(device)
            let name = device.staticTexts["grantedDeviceName"]
            XCTAssertTrue(name.exists)
            XCTAssertFalse(name.label.isEmpty)
            XCTAssertEqual(device.staticTexts["grantedDeviceUnavailable"].exists, index == 0)
            XCTAssertEqual(device.staticTexts["grantedDeviceSystemDisk"].exists, index == 2)
            XCTAssertTrue(device.buttons.firstMatch.exists, "Device path has no disclosure control")
            assertPathsCollapsed()
            assertWithinScreen(name)
            switch index {
            case 0:
                XCTAssertEqual(name.label, "Sonoff Zigbee 3.0")
                shot("02-missing-collapsed")
            case 1:
                XCTAssertTrue(name.label.localizedCaseInsensitiveContains("USB"))
            case 2:
                XCTAssertEqual(name.label, "Samsung SSD 990 PRO 1TB")
                shot("03-system-disk-collapsed")
            case 3:
                XCTAssertEqual(name.label, "WDC WD40EFRX 4TB")
            default:
                XCTAssertNotEqual(name.label, Self.paths[index])
                shot("04-unknown-kind-collapsed")
            }
        }

        reveal(row(0), upward: false)
        tap(row(0).buttons, "Missing serial device path disclosure")
        let path = row(0).staticTexts[Self.serialPath]
        XCTAssertTrue(path.waitForExistence(timeout: 8))
        reveal(path)
        assertWithinScreen(path)
        for hiddenPath in Self.paths.dropFirst() {
            XCTAssertFalse(app.staticTexts[hiddenPath].exists, "Expanding one row exposed another path")
        }
        shot("05-missing-expanded-raw-path")
        tap(row(0).buttons, "Collapse serial device path")
        assertPathsCollapsed()
    }

    private func openHomeHubDetail() {
        tap(app.buttons.matching(UITestSidebar.button), "Sidebar")
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "All")), "All apps")
        tap(app.staticTexts.matching(NSPredicate(format: "label == %@", "Home Hub")), "Home Hub")
        XCTAssertTrue(
            row(0).waitForExistence(timeout: 8),
            "Home Hub real detail has no granted hardware section\n\(app.debugDescription)"
        )
    }

    private func row(_ index: Int) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: "grantedDevice-\(index)").firstMatch
    }

    private func assertPathsCollapsed() {
        for path in Self.paths {
            XCTAssertFalse(app.staticTexts[path].exists, "Raw device path is not collapsed: \(path)")
        }
    }

    private func assertWithinScreen(_ element: XCUIElement) {
        let screen = app.frame
        XCTAssertGreaterThan(element.frame.width, 0)
        XCTAssertGreaterThanOrEqual(element.frame.minX, screen.minX)
        XCTAssertLessThanOrEqual(element.frame.maxX, screen.maxX)
    }

    private func reveal(_ element: XCUIElement, upward: Bool = true) {
        for _ in 0..<8 {
            if element.exists && element.isHittable { return }
            if upward { app.swipeUp() } else { app.swipeDown() }
        }
        XCTAssertTrue(element.exists && element.isHittable, "Element did not become visible\n\(app.debugDescription)")
    }

    private func tap(_ query: XCUIElementQuery, _ name: String) {
        let deadline = Date().addingTimeInterval(8)
        repeat {
            if let element = query.allElementsBoundByIndex.first(where: {
                $0.exists && $0.isHittable && $0.isEnabled
            }) {
                element.tap()
                return
            }
            Thread.sleep(forTimeInterval: 0.2)
        } while Date() < deadline
        XCTFail("\(name) is not tappable\n\(app.debugDescription)")
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "UI-FAKE LAYOUT ONLY-granted-detail-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
