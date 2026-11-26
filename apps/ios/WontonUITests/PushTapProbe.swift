import Foundation
import XCTest

/// 推送横幅点开进对应会话的探针：夜跑没有推送循环，外层带 `WONTON_PUSH_PROBE=1` 时才跑。
/// 前置（外层 e2e.sh）：先用 simctl launch 带配对码配好会话（数据目录固定后缀 pushprobe）并抓下密钥；
/// 推送由外层的 shell 循环推（模拟器 SDK 的 Foundation 没有 Process，测试进程起不了 simctl）。
final class PushTapProbe: XCTestCase {
    private static let dataSuffix = "pushprobe"
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        // 夜跑没有外层推送循环，也没给配对码；只有外层 e2e 带 TEST_RUNNER_WONTON_PUSH_PROBE=1 时才跑。
        guard ProcessInfo.processInfo.environment["WONTON_PUSH_PROBE"] == "1" else {
            throw XCTSkip("推送探针要外层循环和配对码，夜跑不带")
        }
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = Self.dataSuffix
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func springboard() -> XCUIApplication {
        XCUIApplication(bundleIdentifier: "com.apple.springboard")
    }

    /// 通知权限的框：系统弹在 SpringBoard 上。中英文都备着。
    private func acceptPermissionAlertIfUp() -> Bool {
        for title in ["Allow", "允许"] {
            let button = springboard().alerts.buttons[title]
            if button.waitForExistence(timeout: 2) {
                button.tap()
                return true
            }
        }
        return false
    }

    /// 引导页（OnboardingGate 顶替整页）：跳过优先，最后一页的「跳过」是禁用的，按「完成」。
    /// 引导和权限框谁先到就处理谁；只点能点的，防止对禁用的「跳过」打转。轮数封顶：正常三页内走完。
    private func passOnboardingAndPermission() {
        let step = ["Skip", "跳过", "Next", "下一步", "进入聊天", "Done", "完成"]
        for _ in 0..<12 {
            if acceptPermissionAlertIfUp() { continue }
            let enabled = step.lazy
                .compactMap { title -> XCUIElement? in
                    let button = self.app.buttons[title].firstMatch
                    return button.exists && button.isEnabled ? button : nil
                }
                .first
            guard let button = enabled else { break }
            button.tap()
            usleep(1_000_000)
        }
    }

    private func waitChatList() {
        passOnboardingAndPermission()
        // SwiftUI 的列表在可访问性树里不是 collectionView：等 Bot 房间的行（配对好的账号必有）。
        let botRoom = app.staticTexts["Hermes"]
        XCTAssertTrue(botRoom.waitForExistence(timeout: 60), "聊天列表没出来（会话没恢复？）")
        shot("chat-list")
    }

    /// 第一段：走完引导、点掉权限框，等同步，聊天列表在（外层随后推、截图，杀 App 再推）。
    func test1GrantPermissionAndWaitSync() throws {
        // 探针要的就是启动时那个权限框（`start()` 在 SKIP 变量下不问）。
        UITestBackend.launch(app, skipPushAuth: false)
        waitChatList()
    }

    /// 第二段：外层的推送循环正在跑，App 一进后台横幅就来；点开，验收进了房间。
    func test2TapBannerOpensTheRoom() throws {
        UITestBackend.launch(app, skipPushAuth: false)
        waitChatList()
        XCUIDevice.shared.press(.home)
        let banner = springboard().staticTexts["Hermes"]
        XCTAssertTrue(banner.waitForExistence(timeout: 10), "通知横幅没出现")
        shot("banner")
        banner.tap()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15), "点横幅后 App 没回前台")
        // 房间页有输入框，聊天列表没有。
        let composer = app.textFields.firstMatch
        XCTAssertTrue(composer.waitForExistence(timeout: 10), "点开横幅没进房间（没有输入框）")
        shot("after-tap")
    }
}
