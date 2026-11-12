import SwiftUI
import WontonClient

/// Agent 一轮的状态（ADR-0082 的 `state`）：`uncertain` 由核心合成，Bot 离线时还停在
/// `working`、`waiting` 的轮次变成它，不转圈只说不确定；不认识的值落到 `other`。
enum AgentRunRowState: Equatable {
    case working, waiting, done, failed, uncertain, other
}

/// 这一轮怎么结束的（`app.wonton.end`）。`other` 的原文只在 `result` 里，壳不单独画。
enum AgentRunRowEnd: Equatable {
    case cancelled, uncertain
}

/// Agent 自己的提问（MSC4564 的 `choices`）：以主人的身份发反应作答，不经核心确认。
struct AgentRunRowChoice: Equatable {
    let key: String
    let label: String
}

/// 时间线里 Agent 一轮运行状态那一行的数据（ADR-0082）。接线时由调用处从核心的
/// `TimelineContent.AgentRun` 映射。`step`、`result`、`body`、choice 的 `label` 是 Bot 自己写的
/// 文字，原样显示，不翻译、不拼句。
struct AgentRunRowModel: Equatable {
    var state: AgentRunRowState
    /// 当前步骤的一句自述。
    var step: String?
    /// 到现在几步。
    var steps: Int?
    /// 0～1，有计划时才有。
    var progress: Double?
    /// 结束时的一句总结。
    var result: String?
    /// 打断了、结果不确定这两种结束的区分。
    var end: AgentRunRowEnd?
    /// `other` 状态按普通 Bot 提示消息显示的正文。
    var body: String?
    /// waiting 且这个 Bot 在这个房间有等回答的核心卡片：整行可点，打开那张卡片。
    var cardId: String?
    /// Bot 自己的提问。
    var choices: [AgentRunRowChoice]
    /// 发这条状态的 Bot 的名字，choices 区标明是谁在问。
    var senderName: String

    init(
        state: AgentRunRowState,
        step: String? = nil,
        steps: Int? = nil,
        progress: Double? = nil,
        result: String? = nil,
        end: AgentRunRowEnd? = nil,
        body: String? = nil,
        cardId: String? = nil,
        choices: [AgentRunRowChoice] = [],
        senderName: String = ""
    ) {
        self.state = state
        self.step = step
        self.steps = steps
        self.progress = progress
        self.result = result
        self.end = end
        self.body = body
        self.cardId = cardId
        self.choices = choices
        self.senderName = senderName
    }
}

/// 时间线里 Agent 一轮运行状态的那一行（ADR-0082）：真的在干活才转圈，要人管的醒目，
/// 打断了和结果不确定不用失败的红。减弱动态效果时不转圈。
struct AgentRunRow: View {
    let run: AgentRunRowModel
    /// 点开这张等回答的确认卡片（waiting 且有卡片 ID 时才有）。
    let openCard: (String) -> Void
    /// 对 Bot 自己的提问以主人的身份发反应作答（choices 的 key）。
    let answerChoice: (String) -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            status
            if run.state == .working, let progress = run.progress {
                ProgressView(value: progress)
                    .progressViewStyle(.linear)
                    .tint(Color(colors.accentInk))
            }
            if !run.choices.isEmpty {
                asking
            }
        }
        .padding(.trailing, WontonTheme.spacing.s48)
        .frame(maxWidth: .infinity, alignment: .leading)
        // 读屏和界面测试用的稳定标识，跟着核心的键走，不跟语言走。
        .accessibilityIdentifier(identifier)
    }

    /// waiting 且有等回答的核心卡片时整行可点（ADR-0043：能跳转的地方方便跳转）。
    @ViewBuilder private var status: some View {
        if let cardId = run.cardId, run.state == .waiting {
            Button {
                openCard(cardId)
            } label: {
                line
                    .frame(minHeight: WontonTheme.size.target)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        } else {
            line
        }
    }

    private var line: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            mark
            if !text.isEmpty {
                Text(text)
                    .wontonText(run.state == .done ? WontonTheme.typography.caption : WontonTheme.typography.supporting)
                    .foregroundStyle(Color(ink))
                    .multilineTextAlignment(.leading)
            }
            if run.cardId != nil, run.state == .waiting {
                // 装饰：整行可点，箭头只表示能过去。
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.accentInk))
            }
        }
    }

    @ViewBuilder private var mark: some View {
        switch run.state {
        case .working:
            if reduceMotion {
                WontonIcon.agent.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                ProgressView().controlSize(.small).tint(Color(colors.accentInk))
            }
        case .waiting:
            WontonIcon.notify.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.accentInk))
        case .done:
            WontonIcon.checkCircleFill.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.success))
        case .failed:
            switch run.end {
            case .cancelled:
                WontonIcon.pauseFill.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
            case .uncertain:
                WontonIcon.help.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.warning))
            case nil:
                WontonIcon.error.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.danger))
            }
        case .uncertain:
            WontonIcon.help.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.warning))
        case .other:
            EmptyView()
        }
    }

    /// 状态行的文字。打断了、结果不确定用壳自己的文字，其余显示 Bot 的自述。
    private var text: String {
        switch run.state {
        case .working:
            return run.step ?? ""
        case .waiting:
            return String(localized: .AgentRun.agentRunWaiting)
        case .done:
            if let steps = run.steps {
                return String(localized: .AgentRun.agentRunDoneSteps(steps))
            }
            return String(localized: .AgentRun.agentRunDone)
        case .failed:
            switch run.end {
            case .cancelled:
                return String(localized: .AgentRun.agentRunCancelled)
            case .uncertain:
                return String(localized: .AgentRun.agentRunUncertain)
            case nil:
                return run.result ?? ""
            }
        case .uncertain:
            return String(localized: .AgentRun.agentRunUncertain)
        case .other:
            return run.body ?? ""
        }
    }

    private var ink: Rgb {
        switch run.state {
        case .working, .other:
            return colors.textSecondary
        case .waiting:
            return colors.accentInk
        case .done:
            return colors.success
        case .failed:
            switch run.end {
            case .cancelled: return colors.textSecondary
            case .uncertain: return colors.warning
            case nil: return colors.danger
            }
        case .uncertain:
            return colors.warning
        }
    }

    /// choices 区：来源行标明是这个 Bot 在问，按钮是描边胶囊——和核心确认卡片的实心按钮分开。
    private var asking: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonIcon.agent.view(size: WontonTheme.size.iconSmall)
                Text(String(localized: .AgentRun.agentRunAsking(run.senderName)))
                    .wontonText(WontonTheme.typography.supporting)
            }
            .foregroundStyle(Color(colors.accentInk))
            HStack(spacing: WontonTheme.spacing.s8) {
                ForEach(run.choices, id: \.key) { choice in
                    Button {
                        answerChoice(choice.key)
                    } label: {
                        Text(choice.label)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.accentInk))
                            .padding(.horizontal, WontonTheme.spacing.s12)
                            .frame(minHeight: WontonTheme.size.chip)
                            .overlay(Capsule().strokeBorder(Color(colors.accentInk), lineWidth: WontonTheme.size.hairline))
                            .frame(minHeight: WontonTheme.size.target)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("agentRun-choice-\(choice.key)")
                }
            }
        }
    }

    /// 界面测试用的稳定标识（同 BotActivity.identifier 的做法）。
    private var identifier: String {
        switch run.state {
        case .working: "agentRun-working"
        case .waiting: "agentRun-waiting"
        case .done: "agentRun-done"
        case .failed:
            switch run.end {
            case .cancelled: "agentRun-cancelled"
            case .uncertain: "agentRun-uncertain"
            case nil: "agentRun-failed"
            }
        case .uncertain: "agentRun-uncertain"
        case .other: "agentRun-other"
        }
    }
}

#if DEBUG
/// 走查和 Xcode 预览用的固定数据。文字是 Bot 自述的样子，壳不加工。
enum AgentRunFixtures {
    /// 位置参数构造：夹具的字面量不落在 `label:` 参数位上（界面文字检查的规则二）。
    private static func choice(_ key: String, _ text: String) -> AgentRunRowChoice {
        AgentRunRowChoice(key: key, label: text)
    }

    static let working = AgentRunRowModel(state: .working, step: "Reading files")
    static let workingWithProgress = AgentRunRowModel(
        state: .working, step: "Running command (step 3)", progress: 0.4
    )
    static let waitingForCard = AgentRunRowModel(state: .waiting, cardId: "card-1", senderName: "Claude Code")
    static let waitingWithChoices = AgentRunRowModel(
        state: .waiting,
        choices: [
            choice("yes", "Yes, go ahead"),
            choice("no", "No, stop"),
        ],
        senderName: "Hermes"
    )
    static let doneWithSteps = AgentRunRowModel(state: .done, steps: 5)
    static let done = AgentRunRowModel(state: .done)
    static let failed = AgentRunRowModel(state: .failed, result: "Model gateway error")
    static let cancelled = AgentRunRowModel(state: .failed, end: .cancelled)
    static let uncertain = AgentRunRowModel(state: .failed, end: .uncertain)
    static let offline = AgentRunRowModel(state: .uncertain)
    static let other = AgentRunRowModel(state: .other, body: "Runner restarted, this run is inconclusive")

    static var all: [(String, AgentRunRowModel)] {
        [
            ("working", working), ("working with progress", workingWithProgress),
            ("waiting (card)", waitingForCard), ("waiting (choices)", waitingWithChoices),
            ("done", doneWithSteps), ("done (no steps)", done),
            ("failed", failed), ("cancelled", cancelled), ("uncertain", uncertain),
            ("offline", offline), ("other", other),
        ]
    }
}

#Preview("Agent run states") {
    ScrollView {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            ForEach(AgentRunFixtures.all, id: \.0) { _, run in
                AgentRunRow(
                    run: run,
                    openCard: { _ in },
                    answerChoice: { _ in }
                )
            }
        }
        .padding(WontonTheme.spacing.s16)
    }
}
#endif
