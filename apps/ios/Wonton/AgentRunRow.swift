import SwiftUI
import WontonClient

/// 时间线里 Agent 一轮运行状态的那一行（ADR-0082）：真的在干活才转圈，要人管的醒目，
/// 打断了和结果不确定不用失败的红。减弱动态效果时不转圈。
/// 做完了的一轮只留一行小的步骤摘要，点开是 Bot 自己的总结；没有步数也没有总结的不占地方（`isBlank`）。
/// 调过的工具（ADR-0128）默认收起：状态行下面的“N 步”点开才露出清单，做完的一轮清单和总结一起出。
/// 字段就是核心 `TimelineContent.AgentRun` 的字段，用的也是核心的 `AgentRunState`、`AgentRunEnd`
/// 和选项类型；`step`、`result`、`body`、choice 的 `label` 是 Bot 自己写的文字，原样显示，
/// 不翻译、不拼句；不认识的 `state` 落到通用样子，按 `body` 显示。
struct AgentRunRow: View {
    let state: AgentRunState
    /// 当前步骤的一句自述。
    let step: String?
    /// 到现在几步。
    let steps: UInt32?
    /// 0～1，有计划时才有。
    let progress: Float?
    /// 结束时的一句总结。
    let result: String?
    /// 打断了、结果不确定这两种结束的区分。
    let end: AgentRunEnd?
    /// 不认识的状态按普通 Bot 提示消息显示的正文。视图自身有 `body`，这里换个名字。
    let bodyText: String
    /// waiting 且这个 Bot 在这个房间有等回答的核心卡片：整行可点，打开那张卡片。
    let cardId: String?
    /// Bot 自己的提问。
    let choices: [AgentRunChoice]
    /// 发这条状态的 Bot 的名字，choices 区标明是谁在问。
    let senderName: String
    /// 这条状态消息自己的发送时间（Unix 毫秒，编辑不改它）；working 行从它算已经等了多久。
    let timestampMs: UInt64
    /// 这一轮调过的工具，`title` 是 Bot 写的说明。
    var trace: [AgentToolCall] = []
    /// 点开这张等回答的确认卡片（waiting 且有卡片 ID 时才有）。
    let openCard: (String) -> Void
    /// 对 Bot 自己的提问以主人的身份发反应作答（choices 的 key）。
    let answerChoice: (String) -> Void
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    /// 做完了、没有步数也没有总结的一轮没有可写的：时间线不给它留行。
    nonisolated static func isBlank(state: AgentRunState, steps: UInt32?, result: String?) -> Bool {
        state == .done && (steps ?? 0) == 0 && (result ?? "").isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            if state == .done {
                summary
            } else {
                status
                if !isOther, !trace.isEmpty {
                    AgentRunSteps(total: Int(steps ?? UInt32(trace.count)), expandable: true, expanded: $expanded)
                    if expanded {
                        AgentRunTrace(calls: trace)
                    }
                }
            }
            if state == .working, let progress {
                ProgressView(value: Double(progress))
                    .progressViewStyle(.linear)
                    .tint(Color(colors.accentInk))
            }
            if !choices.isEmpty, !isOther {
                asking
            }
        }
        .padding(.trailing, WontonTheme.spacing.s48)
        .frame(maxWidth: .infinity, alignment: .leading)
        // 读屏和界面测试用的稳定标识，跟着核心的键走，不跟语言走。先包成容器：不然整行的标识会盖掉里面
        // “已等待”那一行自己的标识。
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(identifier)
    }

    /// 做完的一轮：一行“N 步”，有清单或总结时能点开；没有步数只有总结时直接写总结。
    @ViewBuilder private var summary: some View {
        let total = Int(steps ?? 0)
        let result = result ?? ""
        if total > 0 {
            AgentRunSteps(total: total, expandable: !result.isEmpty || !trace.isEmpty, expanded: $expanded)
            if expanded {
                if !trace.isEmpty {
                    AgentRunTrace(calls: trace)
                }
                if !result.isEmpty {
                    resultText(result)
                }
            }
        } else if !result.isEmpty {
            resultText(result)
        }
    }

    private func resultText(_ result: String) -> some View {
        Text(result)
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(colors.textSecondary))
            .multilineTextAlignment(.leading)
    }

    /// 不认识的状态值：按普通 Bot 提示消息显示 `body`，不画标记和按钮。
    private var isOther: Bool {
        if case .other = state { true } else { false }
    }

    /// waiting 且有等回答的核心卡片时整行可点（ADR-0043：能跳转的地方方便跳转）。
    @ViewBuilder private var status: some View {
        if let cardId, state == .waiting {
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
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(ink))
                    .multilineTextAlignment(.leading)
            }
            if showsWaited {
                // 只有 working 行带时长：有它才撑满整行，把时长顶到右边；别的行保持原宽。
                Spacer(minLength: 0)
                waited
            }
            if cardId != nil, state == .waiting {
                // 装饰：整行可点，箭头只表示能过去。
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.accentInk))
            }
        }
    }

    /// working 且带发送时间才显示时长；时间戳为 0 是夹具的“不显示”。
    private var showsWaited: Bool {
        state == .working && timestampMs > 0
    }

    /// 已等待的分钟数（ADR-0082 §4）：从状态消息自己的发送时间算（编辑不改它），
    /// TimelineView 每分钟走一次；不满一分钟不写，不凑数。
    private var waited: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            let elapsedMs = context.date.timeIntervalSince1970 * 1_000 - Double(timestampMs)
            let minutes = Int(elapsedMs / 60_000)
            if minutes >= 1 {
                Text(.AgentRun.agentRunWaited(minutes))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .accessibilityIdentifier("agentRun-elapsed")
            }
        }
    }

    @ViewBuilder private var mark: some View {
        switch state {
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
        case .failed:
            failureMark
        case .uncertain:
            WontonIcon.help.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.warning))
        case .done, .other:
            EmptyView()
        }
    }

    /// 失败按结束原因分开：打断和不确定不用失败的红，以后新加的结束原因按普通失败画。
    @ViewBuilder private var failureMark: some View {
        switch end {
        case .cancelled:
            WontonIcon.pauseFill.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
        case .uncertain:
            WontonIcon.help.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.warning))
        case .other, nil:
            WontonIcon.error.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.danger))
        }
    }

    /// 状态行的文字。打断了、结果不确定用壳自己的文字，其余显示 Bot 的自述。
    private var text: String {
        switch state {
        case .working:
            return step ?? ""
        case .waiting:
            return String(localized: .AgentRun.agentRunWaiting)
        case .failed:
            switch end {
            case .cancelled:
                return String(localized: .AgentRun.agentRunCancelled)
            case .uncertain:
                return String(localized: .AgentRun.agentRunUncertain)
            case .other, nil:
                return result ?? ""
            }
        case .uncertain:
            return String(localized: .AgentRun.agentRunUncertain)
        case .done:
            return ""
        case .other:
            return bodyText
        }
    }

    private var ink: Rgb {
        switch state {
        case .working, .done, .other:
            return colors.textSecondary
        case .waiting:
            return colors.accentInk
        case .failed:
            switch end {
            case .cancelled: return colors.textSecondary
            case .uncertain: return colors.warning
            case .other, nil: return colors.danger
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
                Text(String(localized: .AgentRun.agentRunAsking(senderName)))
                    .wontonText(WontonTheme.typography.supporting)
            }
            .foregroundStyle(Color(colors.accentInk))
            AgentRunChoices(choices: choices, state: state, answer: answerChoice)
        }
    }

    /// 界面测试用的稳定标识（同 BotActivity.identifier 的做法）。
    private var identifier: String {
        switch state {
        case .working: "agentRun-working"
        case .waiting: "agentRun-waiting"
        case .done: "agentRun-done"
        case .failed:
            switch end {
            case .cancelled: "agentRun-cancelled"
            case .uncertain: "agentRun-uncertain"
            case .other, nil: "agentRun-failed"
            }
        case .uncertain: "agentRun-uncertain"
        case .other: "agentRun-other"
        }
    }
}

#if DEBUG
#Preview("Agent run states") {
    ScrollView {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            ForEach(AgentRunFixtures.all, id: \.0) { _, run in
                run
            }
        }
        .padding(WontonTheme.spacing.s16)
    }
}
#endif
