import SwiftUI
import WontonClient

/// 状态行上的一串待回答选项（ADR-0082）：一排最多两个，放不下就换排，每排等分宽度，
/// 照 Telegram 的内联键盘（ADR-0109 第 7 节）。一排只有一个时铺满整排；比自己那一格还长的字截断。
struct AgentRunChoices: View {
    let choices: [AgentRunChoice]
    let answer: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ChoiceRowsLayout(spacing: WontonTheme.spacing.s8) {
            ForEach(choices, id: \.key) { choice in
                Button {
                    answer(choice.key)
                } label: {
                    label(choice)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("agentRun-choice-\(choice.key)")
            }
        }
    }

    private func label(_ choice: AgentRunChoice) -> some View {
        Text(choice.label)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.accentInk))
            .lineLimit(1)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.chip)
            .overlay(
                Capsule().strokeBorder(Color(colors.accentInk), lineWidth: WontonTheme.size.hairline)
            )
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
    }
}

/// 按内容宽度贪心分排，再让每一排平分容器宽度。`Layout` 而不是 `HStack`：`HStack` 只摊开当前这一排，
/// 窄屏或大字号下每个选项都缩到一两个字宽、竖着堆。
private struct ChoiceRowsLayout: Layout {
    let spacing: CGFloat
    /// ADR-0109：一排最多两个。
    private let perRow = 2

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.replacingUnspecifiedDimensions().width
        let rows = rows(width: width, subviews: subviews)
        let height = rows.enumerated().reduce(CGFloat.zero) { total, row in
            total + rowHeight(row.element, subviews: subviews) + (row.offset == 0 ? 0 : spacing)
        }
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var cursorY = bounds.minY
        for row in rows(width: bounds.width, subviews: subviews) {
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

    private func rows(width: CGFloat, subviews: Subviews) -> [[Int]] {
        var rows: [[Int]] = [[]]
        var used: CGFloat = 0
        for index in subviews.indices {
            let natural = subviews[index].sizeThatFits(.unspecified).width
            let row = rows[rows.count - 1]
            let next = row.isEmpty ? natural : used + spacing + natural
            if row.isEmpty || (row.count < perRow && next <= width) {
                rows[rows.count - 1].append(index)
                used = next
            } else {
                rows.append([index])
                used = natural
            }
        }
        return rows
    }

    private func rowHeight(_ row: [Int], subviews: Subviews) -> CGFloat {
        row.reduce(CGFloat.zero) { max($0, subviews[$1].sizeThatFits(.unspecified).height) }
    }
}
