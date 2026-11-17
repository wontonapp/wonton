#if DEBUG
import SwiftUI
import WontonClient

/// Checking 不盖住聊天页；UI 测试不能把侧边栏存在当成交接判定已完成。
struct KeyHandoffTestState: View {
    let model: ChatModel

    var body: some View {
        if ProcessInfo.processInfo.environment["WONTON_UI_TEST_STATE"] == "1" {
            Color.clear
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("key-handoff-test-state")
                .accessibilityLabel("key-handoff-test-state")
                .accessibilityValue(state)
                .allowsHitTesting(false)
        }
    }

    private var state: String {
        guard case .signedIn = model.viewModel.account,
              case .running = model.viewModel.sync else { return "connecting" }
        guard model.viewModel.encryption.recovery != .unknown else { return "checking" }
        switch model.viewModel.encryption.keyHandoff {
        case .checking: return "checking"
        case .receiving: return "receiving"
        case nil: return "settled"
        case .giving: return "giving"
        case .cannotGive: return "cannot-give"
        case .other: return "other"
        }
    }
}
#endif
