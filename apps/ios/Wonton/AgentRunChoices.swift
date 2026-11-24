import SwiftUI
import WontonClient

/// 状态行上的一串待回答选项（ADR-0082）：一排最多两个，多出来的换到下一排，每排等分宽度，
/// 照 Telegram 的内联键盘（ADR-0109 第 8 节）。一排只有一个时铺满整排；比自己那一格还长的字截断。
struct AgentRunChoices: View {
    let choices: [AgentRunChoice]
    /// 状态行只在等待时有效：跑完了、失败了画成灰的、点不动（ADR-0109 第 8 节）。
    let state: AgentRunState
    let answer: (String) -> Void
    @Environment(\.wontonColors) private var colors

    private var answered: Bool { choices.contains { $0.chosen } }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            ChoiceRowsLayout(spacing: WontonTheme.spacing.s8) {
                ForEach(choices, id: \.key) { choice in
                    let canAnswer = state == .waiting && !answered && choice.send != .sending
                    Button {
                        answer(choice.key)
                    } label: {
                        label(choice, canAnswer: canAnswer)
                    }
                    .buttonStyle(.plain)
                    .disabled(!canAnswer)
                    .accessibilityIdentifier("agentRun-choice-\(choice.key)")
                }
            }
            // 没发出去：和消息选项一样先给一句“没发出去，再点一次重试”，原因收在详情里。
            if let failure = failure, case let .failed(reason) = failure {
                Text(.Message0.messageChoiceFailed)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.danger))
                ProblemDetail(text: reason)
            }
        }
    }

    private var failure: ChoiceSend? {
        choices.map(\.send).first { if case .failed = $0 { true } else { false } }
    }

    private func label(_ choice: AgentRunChoice, canAnswer: Bool) -> some View {
        let ink = choice.chosen
            ? Color(colors.textOnAccentFill)
            : canAnswer ? Color(colors.accentInk) : Color(colors.textSecondary)
        return HStack(spacing: WontonTheme.spacing.s4) {
            Text(choice.label)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(ink)
                .lineLimit(1)
            if choice.send == .sending {
                ProgressView().controlSize(.mini).tint(ink)
            } else if case .failed = choice.send {
                Text(verbatim: "!").foregroundStyle(Color(colors.danger))
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.chip)
        .background(
            choice.chosen ? Color(colors.accentFill) : .clear, in: Capsule()
        )
        .overlay(
            Capsule().strokeBorder(ink, lineWidth: WontonTheme.size.hairline)
        )
        .frame(minHeight: WontonTheme.size.target)
        .contentShape(Rectangle())
    }
}

/// 按个数两两分排，再让每一排平分容器宽度。`Layout` 而不是 `HStack`：`HStack` 只摊开当前这一排，
/// 窄屏或大字号下每个选项都缩到一两个字宽、竖着堆。
private struct ChoiceRowsLayout: Layout {
    let spacing: CGFloat
    /// ADR-0109：一排最多两个。
    private let perRow = 2

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.replacingUnspecifiedDimensions().width
        let rows = rows(count: subviews.count)
        let height = rows.enumerated().reduce(CGFloat.zero) { total, row in
            total + rowHeight(row.element, subviews: subviews) + (row.offset == 0 ? 0 : spacing)
        }
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var cursorY = bounds.minY
        for row in rows(count: subviews.count) {
            let itemWidth = (bounds.width - spacing * CGFloat(row.count - 1)) / CGFloat(row.count)
            var cursorX = bounds.minX
            for index in row {
                let height = subviews[index].sizeThatFits(.unspecified).height
                subviews[index].place(
                    at: CGPoint(x: cursorX, y: cursorY),
                    anchor: .topLeading,
                    proposal: ProposedViewSize(width: itemWidth, height: height)
                )
                cursorX += itemWidth + spacing
            }
            cursorY += rowHeight(row, subviews: subviews) + spacing
        }
    }

    /// 只按个数分排，不按文字宽度：大字号下每格的自然宽度都超过半屏，按宽度判就会退成一格一排、满屏竖着堆；
    /// 该截断的字交给按钮的 `lineLimit(1)`。
    private func rows(count: Int) -> [[Int]] {
        stride(from: 0, to: count, by: perRow).map {
            Array($0..<min($0 + perRow, count))
        }
    }

    private func rowHeight(_ row: [Int], subviews: Subviews) -> CGFloat {
        row.reduce(CGFloat.zero) { max($0, subviews[$1].sizeThatFits(.unspecified).height) }
    }
}
