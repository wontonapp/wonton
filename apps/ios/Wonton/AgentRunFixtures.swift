#if DEBUG
// 只改壳的 PR 不编核心：核心产物按内容哈希取，见 .github/workflows/README.md 的“核心产物”。
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
            ("done (summary)", doneWithSummary),
            ("done (no steps)", done),
            ("failed", failed),
            ("cancelled", cancelled),
            ("uncertain", uncertain),
            ("offline", offline),
            ("other", other),
            ("working (trace)", workingWithTrace)
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
        senderName: String = "",
        timestampMs: UInt64 = 0,
        trace: [AgentToolCall] = []
    ) -> AnyView {
        AnyView(AgentRunRow(
            state: state,
            step: step,
            steps: steps,
            progress: progress,
            result: result,
            end: end,
            bodyText: body,
            cardId: cardId,
            choices: choices,
            senderName: senderName,
            // 0 = 不带发送时间：已等待那行不显示，参考图不吃真实时钟。
            timestampMs: timestampMs,
            trace: trace,
            openCard: { _ in },
            answerChoice: { _ in }
        ))
    }

    /// 位置参数构造：夹具的字面量不落在 `label:` 参数位上（界面文字检查的规则二）。
    private static func choice(_ key: String, _ label: String) -> AgentRunChoice {
        AgentRunChoice(key: key, label: label, chosen: false, send: .idle)
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
                choice("no", "No, stop")
            ],
            senderName: "Hermes"
        )
    }
    static var doneWithSteps: AnyView { row(state: .done, steps: 5) }
    static var doneWithSummary: AnyView {
        row(state: .done, steps: 3, result: "Updated the config and restarted the service")
    }
    static var done: AnyView { row(state: .done) }
    static var failed: AnyView { row(state: .failed, result: "Model gateway error") }
    static var cancelled: AnyView { row(state: .failed, end: .cancelled) }
    static var uncertain: AnyView { row(state: .failed, end: .uncertain) }
    static var offline: AnyView { row(state: .uncertain) }
    static var other: AnyView {
        row(state: .other("new-state"), body: "Runner restarted, this run is inconclusive")
    }
    static var workingWithTrace: AnyView {
        row(state: .working, step: "Running a command", steps: 2, trace: [
            .sample("c1", .read, "read_file: notes.md", .completed),
            .sample("c2", .execute, "terminal: ls -la", .inProgress)
        ])
    }
}
#endif

extension AgentToolCall {
    /// 样例里 Bot 写的原话：App 原样显示，不是界面文字，不进字符串目录。
    static func sample(_ id: String, _ kind: AgentToolKind, _ said: String, _ status: AgentToolStatus) -> AgentToolCall {
        AgentToolCall(id: id, kind: kind, title: said, status: status)
    }
}
