#if DEBUG
import SwiftUI
import WontonClient

/// 夜跑界面测试的夹具屏（ADR-0082）：不走主界面，直接摆 Agent 状态行。启动参数带
/// `wonton-agent-run-preview` 时生效（同 `QualityPreviews` 的做法）。
/// 时间线要登录后的房间才有（核心假数据只够首屏），这里核对行本身的显示和“点开卡片”的接线；
/// 一轮从 working 到 done 用按钮推进，waiting 行点了记下开卡片这件事；
/// 一串长选项核对多出来的换到下一排（ADR-0109 的选项排法）。
enum AgentRunPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-agent-run-preview")
    }

    /// 启动参数再加 `wonton-agent-run-preview-narrow` 时把夹具收窄成最窄 iPhone 的屏宽：
    /// CI 用大屏机器，窄屏排法要自己造出宽度来。
    static var narrow: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-agent-run-preview-narrow")
    }

    static let narrowWidth: CGFloat = 320

    static var root: some View {
        AgentRunPreviewScreen()
    }
}

private struct AgentRunPreviewScreen: View {
    @State private var finished = false
    @State private var openedCard = false
    @State private var answered: String?

    var body: some View {
        ScrollView {
            content
                .padding(WontonTheme.spacing.s16)
                .frame(maxWidth: AgentRunPreview.narrow ? AgentRunPreview.narrowWidth : .infinity, alignment: .leading)
        }
    }

    /// 夹具的字面量不落在 `label:` 参数位上（界面文字检查的规则二，同 `AgentRunFixtures.choice`）。
    private func choice(_ key: String, _ label: String) -> AgentRunChoice {
        AgentRunChoice(key: key, label: label, chosen: false, send: .idle)
    }

    private var content: some View {
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
                // 十分钟前发出的：已等待显示 10 分钟，走屏跑完也跨不进 11。
                timestampMs: UInt64(Date.now.timeIntervalSince1970 * 1_000) - 600_000,
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
                timestampMs: 0,
                openCard: { _ in openedCard = true },
                answerChoice: { _ in }
            )
            // Hermes 审批那种一串选项：宽过一行时换成多排（ADR-0082 的选项按钮）。
            AgentRunRow(
                state: .waiting,
                step: nil,
                steps: nil,
                progress: nil,
                result: nil,
                end: nil,
                bodyText: "",
                cardId: nil,
                choices: [
                    choice("once", "Allow once"),
                    choice("session", "Allow for this session"),
                    choice("always", "Allow for this Bot always"),
                    choice("deny", "Deny")
                ],
                senderName: "Hermes",
                timestampMs: 0,
                openCard: { _ in },
                answerChoice: { answered = $0 }
            )
            if let answered {
                Text(verbatim: "answered \(answered)")
                    .accessibilityIdentifier("fixture-answered")
            }
            if openedCard {
                // 界面测试断“点开了卡片”的占位；真的打开是时间线把那张卡片滚进来。
                Color.clear
                    .frame(height: 200)
                    .accessibilityElement()
                    .accessibilityIdentifier("fixture-card")
            }
        }
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
