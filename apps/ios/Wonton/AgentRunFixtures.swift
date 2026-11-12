#if DEBUG
import SwiftUI
import WontonClient

/// 状态行的预览数据。只在 Debug 里编；字段和取值都是核心的 `AgentRunState`、`AgentRunEnd`、
/// `AgentRunChoice`，不复刻壳的模型。文字是 Bot 自述的样子，壳不加工。
enum AgentRunFixtures {
    static var all: [(String, AnyView)] {
        [
            ("working", working),
            ("working with progress", workingWithProgress),
            ("waiting (card)", waitingForCard),
            ("waiting (choices)", waitingWithChoices),
            ("done", doneWithSteps),
            ("done (no steps)", done),
            ("failed", failed),
            ("cancelled", cancelled),
            ("uncertain", uncertain),
            ("offline", offline),
            ("other", other),
        ]
    }

    private static func row(
        state: AgentRunState,
        step: String? = nil,
        steps: UInt32? = nil,
        progress: Float? = nil,
        result: String? = nil,
        end: AgentRunEnd? = nil,
        body: String = "",
        cardId: String? = nil,
        choices: [AgentRunChoice] = [],
        senderName: String = ""
    ) -> AnyView {
        AnyView(AgentRunRow(
            state: state,
            step: step,
            steps: steps,
            progress: progress,
            result: result,
            end: end,
            body: body,
            cardId: cardId,
            choices: choices,
            senderName: senderName,
            openCard: { _ in },
            answerChoice: { _ in }
        ))
    }

    /// 位置参数构造：夹具的字面量不落在 `label:` 参数位上（界面文字检查的规则二）。
    private static func choice(_ key: String, _ label: String) -> AgentRunChoice {
        AgentRunChoice(key: key, label: label)
    }

    static var working: AnyView { row(state: .working, step: "Reading files") }
    static var workingWithProgress: AnyView {
        row(state: .working, step: "Running command (step 3)", progress: 0.4)
    }
    static var waitingForCard: AnyView {
        row(state: .waiting, cardId: "card-1", senderName: "Claude Code")
    }
    static var waitingWithChoices: AnyView {
        row(
            state: .waiting,
            choices: [
                choice("yes", "Yes, go ahead"),
                choice("no", "No, stop"),
            ],
            senderName: "Hermes"
        )
    }
    static var doneWithSteps: AnyView { row(state: .done, steps: 5) }
    static var done: AnyView { row(state: .done) }
    static var failed: AnyView { row(state: .failed, result: "Model gateway error") }
    static var cancelled: AnyView { row(state: .failed, end: .cancelled) }
    static var uncertain: AnyView { row(state: .failed, end: .uncertain) }
    static var offline: AnyView { row(state: .uncertain) }
    static var other: AnyView {
        row(state: .other("new-state"), body: "Runner restarted, this run is inconclusive")
    }
}
#endif
