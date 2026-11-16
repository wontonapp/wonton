import XCTest

/// 安装卡片上选设备（ADR-0036），不要后端（`WONTON_UI_FAKE` 的假数据：`ui_fake_devices.rs` 里“等你确认”的那张卡片）：
/// 串口没选时不能同意；系统盘有标记、硬盘默认不勾；同意以后发回去的是选中的串口和勾上的盘。
/// 假核心把收到的设备 ID 写进卡片的结果，这里照着看。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/DeviceGrantUITests
final class DeviceGrantUITests: XCTestCase {
    private var app: XCUIApplication!
    private let dataSuffix = UUID().uuidString

    private static let serial = "serial:0:/dev/serial/by-id/usb-ITead_Sonoff_Zigbee_3.0-if00-port0"
    private static let systemDisk = "disk:/dev/disk/by-id/nvme-Samsung_SSD_990_PRO_1TB"
    private static let dataDisk = "disk:/dev/disk/by-id/ata-WDC_WD40EFRX-68N32N0"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testChosenDevicesGoBackWithTheAnswer() {
        launch()
        tap(button("confirmations-entry"), "“等你确认”的入口")
        tap(button("confirmation-row-ui-fake-devices"), "带设备的安装卡片")

        XCTAssertTrue(waitHittable(option(Self.serial)), "卡片上没有可选的串口\n\(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["deviceUsbAll"].exists, "卡片上没写 USB 是整类授权")
        XCTAssertTrue(row(Self.systemDisk).staticTexts["deviceSystemDisk"].exists, "系统盘没有标记")
        XCTAssertFalse(row(Self.dataDisk).staticTexts["deviceSystemDisk"].exists, "数据盘被标成了系统盘")
        for id in [Self.serial, Self.systemDisk, Self.dataDisk] {
            XCTAssertFalse(option(id).firstMatch.isSelected, "\(id) 一上来就选中了")
        }
        shot("untouched")

        let install = app.buttons.matching(NSPredicate(format: "label == %@", "Install"))
        XCTAssertFalse(install.firstMatch.isEnabled, "必选的串口没选就能同意")
        tap(option(Self.serial), "串口")
        tap(option(Self.dataDisk), "数据盘")
        XCTAssertTrue(option(Self.serial).firstMatch.isSelected, "点了串口没选中")
        XCTAssertTrue(option(Self.dataDisk).firstMatch.isSelected, "点了数据盘没勾上")
        XCTAssertFalse(option(Self.systemDisk).firstMatch.isSelected, "没点的系统盘被勾上了")
        shot("chosen")

        tap(install, "同意安装")
        let sent = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", Self.serial, Self.dataDisk)
        )
        XCTAssertTrue(sent.firstMatch.waitForExistence(timeout: 8), "回答里没带选中的设备\n\(app.debugDescription)")
        XCTAssertFalse(sent.firstMatch.label.contains(Self.systemDisk), "回答里带上了没勾的系统盘")
        shot("answered")
    }

    // MARK: 助手

    private func launch() {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(
            waitHittable(app.buttons.matching(UITestSidebar.button), timeout: 20), "没进主界面\n\(app.debugDescription)"
        )
    }

    private func button(_ identifier: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier == %@", identifier))
    }

    /// 一项设备那一行：里面有能点的选项和系统盘的标记。
    private func row(_ device: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: "device-\(device)").firstMatch
    }

    private func option(_ device: String) -> XCUIElementQuery {
        row(device).buttons
    }

    /// 同名的元素里点得到的那个：被盖住的页面还在界面树里，按名字取第一个会取到它们。
    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        // 卡片比一屏高时，下面的选项和按钮要先滚上来。
        if !waitHittable(query, timeout: 3) { app.swipeUp() }
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) != nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) != nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
