import XCTest

/// 要后端（开发栈的核心和 Tuwunel）的走屏共用的前置：
/// 夜跑不连后端，没有配对码时整条用例 XCTSkip；手动跑时经 `PAIRING_CODE` 传配对码
/// （也认旧的 `WONTON_DEV_PAIRING_CODE`）。配对码由 `deploy/dev/core/dev.sh pair <地址>` 打出，10 分钟内有效。
enum UITestBackend {
    static var code: String? {
        let env = ProcessInfo.processInfo.environment
        for key in ["PAIRING_CODE", "WONTON_DEV_PAIRING_CODE"] {
            if let value = env[key], !value.isEmpty { return value }
        }
        return nil
    }

    static func require() throws {
        try XCTSkipIf(code == nil, "需要开发栈：没有 PAIRING_CODE")
    }

    /// 每次启动都用一个空的数据目录，再带上配对码：不依赖模拟器上留下的会话。
    static func passPairingCode(to app: XCUIApplication) {
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        if let code {
            app.launchEnvironment["WONTON_DEV_PAIRING_CODE"] = code
        }
    }
}
