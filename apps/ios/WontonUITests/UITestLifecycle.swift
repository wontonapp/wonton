import XCTest

/// 前后台共用的步骤。
enum UITestLifecycle {
    /// 按 HOME、晾 `background` 秒、再点回来。回到前台的头一瞬间界面还在归位，这时合成的点按不生效
    /// （模拟器实测：立刻点胶囊没反应、侧边栏按钮的位置还在变，等 1.5 秒再点就收起）；人手没这么快，
    /// 等停稳再点。计时从回来的那一瞬算起时传 `linger: 0`，归位不折进阈值。
    static func backgroundAndReturn(
        _ app: XCUIApplication, linger: TimeInterval = 1.5, background: TimeInterval = 0,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 5)
        Thread.sleep(forTimeInterval: background)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回来", file: file, line: line)
        Thread.sleep(forTimeInterval: linger)
    }
}
