import XCTest
import UIKit

/// 复制按钮点了要真的写进剪贴板，并且看得出点上了：图标换成对勾、名字换成“已复制”，过一会儿复原（11-27 主人反馈
/// 恢复密钥的复制“点了没反应”）。不要后端，用 `WONTON_UI_FAKE` 的假数据，假核心给一把还没抄下的恢复密钥：
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/CopyFeedbackUITests
final class CopyFeedbackUITests: XCTestCase {
    private var app: XCUIApplication!
    /// 不是真的恢复密钥格式也无妨：壳只负责原样复制。
    private let recoveryKey = "EsTe ui7e sTKe y000 fake"

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_UI_FAKE_RECOVERY_KEY"] = recoveryKey
        // 模拟器上看恢复密钥前的系统验证要输设备密码：Debug 构建按“已验证”处理，只测验证之后的这一屏。
        app.launchEnvironment["WONTON_UI_TEST_SYSTEM_AUTH"] = "pass"
    }

    func testRecoveryKeyCopyFillsClipboardAndShowsCheck() {
        UITestBackend.launch(app)
        XCTAssertTrue(waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)")
        // 有没抄下的密钥时聊天列表顶上挂着“保存恢复密钥”的提醒：截一张看文字离卡片边的留白。
        XCTAssertTrue(
            waitHittable(app.buttons.matching(identifier: "chat-backup-reminder-dismiss")),
            "聊天列表上没有保存恢复密钥的提醒\n\(app.debugDescription)"
        )
        shot("backup-reminder")
        tap(sidebarButton, "侧边栏按钮")
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "Settings")), "侧边栏里的设置")
        tap(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Chat Backup")), "设置里的聊天记录备份")

        // 夜跑的模拟器没设设备密码：密钥进来就显示，没有“查看恢复密钥”按钮；设了密码的才要先过系统验证。
        let key = app.staticTexts.matching(identifier: "recovery-key-value")
        let reveal = app.buttons.matching(identifier: "recovery-key-reveal")
        let copy = app.buttons.matching(identifier: "recovery-key-copy")
        if !waitHittable(copy, timeout: 3) && !waitHittable(key, timeout: 1) {
            tap(reveal, "查看恢复密钥")
        }
        if !waitHittable(copy, timeout: 4) { app.swipeUp() }
        guard waitHittable(copy), let button = hittable(copy) else {
            return XCTFail("备份页没有恢复密钥的复制按钮\n\(app.debugDescription)")
        }
        // 测试进程读别的 App 写的剪贴板内容会弹“允许粘贴”、一直卡着；只看不弹框的元数据：换没换、有没有纯文本。
        let before = UIPasteboard.general.changeCount
        let idle = button.label
        button.tap()

        XCTAssertTrue(
            waitUntil(timeout: 2) { UIPasteboard.general.changeCount != before },
            "点了复制剪贴板没变"
        )
        XCTAssertTrue(
            UIPasteboard.general.types.contains("public.utf8-plain-text"),
            "剪贴板里不是别的 App 能粘贴的纯文本：\(UIPasteboard.general.types)"
        )
        XCTAssertTrue(
            waitUntil(timeout: 1) { button.label != idle },
            "点了复制按钮没有变化（还是“\(idle)”）\n\(app.debugDescription)"
        )
        shot("recovery-key-copied")
        XCTAssertTrue(waitUntil(timeout: 4) { button.label == idle }, "复制的反馈一直没复原\n\(app.debugDescription)")
    }

    // MARK: 助手

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }

    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        waitUntil(timeout: timeout) { self.hittable(query) != nil }
    }

    private func waitUntil(timeout: TimeInterval, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.1)
        }
        return condition()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
