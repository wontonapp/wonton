#if DEBUG
import SwiftUI
import WontonClient

/// 夜跑界面测试的夹具屏（ADR-0082）：不走主界面，直接摆 Agent 状态行。启动参数带
/// `wonton-agent-run-preview` 时生效（同 `QualityPreviews` 的做法）。
/// 时间线要登录后的房间才有（核心假数据只够首屏），这里核对行本身的显示和“点开卡片”的接线；
/// 一轮从 working 到 done 用按钮推进，waiting 行点了记下开卡片这件事。
enum AgentRunPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-agent-run-preview")
    }

    static var root: some View {
        AgentRunPreviewScreen()
    }
}

private struct AgentRunPreviewScreen: View {
    @State private var finished = false
    @State private var openedCard = false

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s24) {
            AgentRunRow(
                state: finished ? .done : .working,
                step: finished ? nil : "Reading files",
                steps: finished ? 5 : 3,
                progress: nil,
                result: nil,
                end: nil,
                bodyText: "",
                cardId: nil,
                choices: [],
                senderName: "Echo",
                openCard: { _ in },
                answerChoice: { _ in }
            )
            advanceButton
            AgentRunRow(
                state: .waiting,
                step: nil,
                steps: 2,
                progress: nil,
                result: nil,
                end: nil,
                bodyText: "",
                cardId: "card-1",
                choices: [],
                senderName: "Echo",
                openCard: { _ in openedCard = true },
                answerChoice: { _ in }
            )
            if openedCard {
                // 界面测试断“点开了卡片”的占位；真的打开是时间线把那张卡片滚进来。
                Color.clear
                    .frame(height: 200)
                    .accessibilityElement()
                    .accessibilityIdentifier("fixture-card")
            }
        }
        .padding(WontonTheme.spacing.s16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 文字检查的规则二：夹具的字面量不经 label: 参数位。
    private func advance(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Text(verbatim: title) }
            .accessibilityIdentifier("fixture-advance")
    }

    private var advanceButton: some View {
        advance("advance") { finished = true }
    }
}
#endif
