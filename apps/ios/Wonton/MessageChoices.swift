import SwiftUI
import WontonClient

/// 消息下面的选项按钮（ADR-0109 第 8 节）：每行两个、等分宽度，单出来的一个占满一行。
/// 表情和文字原样画核心给的，壳不认任何表情的意思；能不能点、选了哪个也只看核心给的状态。
struct MessageChoicesView: View {
    let choices: MessageChoices
    /// 点了一个选项（参数是它的 `key`）；发送失败的再点一次就是重发。
    let answer: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let rows = choices.rows
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                HStack(spacing: WontonTheme.spacing.s4) {
                    ForEach(Array(row.enumerated()), id: \.element.key) { column, option in
                        ChoiceButton(
                            option: option, state: choices.state,
                            shape: shape(
                                isLastRow: index + 1 == rows.count,
                                isFirst: column == 0, isLast: column + 1 == row.count
                            )
                        ) { answer(option.key) }
                    }
                }
            }
            if let reason = failure {
                Text(.Message0.messageChoiceFailed)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.danger))
                ProblemDetail(text: reason)
            }
        }
    }

    /// 没发出去的那个选项的原因，收在“详情”里。
    private var failure: String? {
        for option in choices.options {
            if case let .failed(message) = option.send { return message }
        }
        return nil
    }

    /// 最下面一行靠外的两个角跟气泡的大圆角，其余是小圆角。
    private func shape(isLastRow: Bool, isFirst: Bool, isLast: Bool) -> UnevenRoundedRectangle {
        let small = WontonTheme.radii.bubbleJoined
        let large = WontonTheme.radii.bubble
        return UnevenRoundedRectangle(
            topLeadingRadius: small,
            bottomLeadingRadius: isLastRow && isFirst ? large : small,
            bottomTrailingRadius: isLastRow && isLast ? large : small,
            topTrailingRadius: small
        )
    }
}

private struct ChoiceButton: View {
    let option: MessageChoice
    let state: ChoicesState
    let shape: UnevenRoundedRectangle
    let action: () -> Void
    @Environment(\.wontonColors) private var colors
    @Environment(\.chatPalette) private var palette

    /// 能点：这组在等我选，而且这一个不在发。发送失败的能再点。
    private var canAnswer: Bool { state == .open && option.send != .sending }
    /// 我选了别的：变灰。
    private var isPassedOver: Bool { state == .answered && !option.chosen }

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s4) {
                ChoiceLabel(option: option)
                progress
            }
            // 没选的是半透明的入气泡色、`primary` 的字；选中的换成 `primary` 的实底（ADR-0109 第 9 节、ADR-0130）。
            .foregroundStyle(Color(option.chosen ? colors.onPrimary : colors.primary))
            .padding(.horizontal, WontonTheme.spacing.s8)
            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.choiceButton)
            .background(shape.fill(option.chosen ? Color(colors.primary) : palette.choiceFill).chatLift(palette))
            .opacity(isPassedOver ? WontonTheme.ratio.secondaryContent : 1)
            .contentShape(Rectangle())
        }
        .buttonStyle(ChoiceButtonStyle())
        .disabled(!canAnswer)
        .accessibilityIdentifier("messageChoice.\(option.key)")
        .accessibilityAddTraits(option.chosen ? .isSelected : [])
    }

    @ViewBuilder private var progress: some View {
        switch option.send {
        case .sending:
            ProgressView().controlSize(.small)
        case .failed:
            WontonIcon.warningCircleFill.view(size: WontonTheme.size.iconTiny)
                .foregroundStyle(Color(colors.danger))
        case .idle, .other:
            // 选中只靠强调色的底：半格宽里再加一个勾，“approve once”这样的字就放不下了。
            EmptyView()
        }
    }
}

/// 系统的按钮样式会把不能点的按钮整个变淡，选中的那个也跟着淡了：这里只在按下时变淡，灰不灰由状态定。
private struct ChoiceButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.opacity(configuration.isPressed ? WontonTheme.ratio.secondaryContent : 1)
    }
}

/// 按钮上的字：表情，后面跟核心从正文里取的那一行；没有就只画表情。
private struct ChoiceLabel: View {
    let option: MessageChoice

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            Text(option.key)
            if let label = option.label, !label.isEmpty {
                Text(label).lineLimit(1)
            }
        }
        .wontonText(WontonTheme.typography.choice)
    }
}

/// 垫在气泡正文底下、看不见的一行：带选项的气泡至少和一行按钮的字一样宽，按钮上的字才不被截断。
struct ChoicesWidthReserve: View {
    let choices: MessageChoices

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(choices.rows.enumerated()), id: \.offset) { _, row in
                // 一行里的按钮等分宽度：每一格都按这一行最宽的那个算。
                HStack(spacing: WontonTheme.spacing.s4) {
                    ForEach(row, id: \.key) { _ in
                        ZStack {
                            ForEach(row, id: \.key) { option in ChoiceLabel(option: option) }
                        }
                        .padding(.horizontal, WontonTheme.spacing.s8)
                    }
                }
                .frame(height: 0)
            }
        }
        .hidden()
        .accessibilityHidden(true)
    }
}

private extension MessageChoices {
    /// 每行两个；单出来的一个自己一行。
    var rows: [[MessageChoice]] {
        stride(from: 0, to: options.count, by: 2).map { Array(options[$0..<min($0 + 2, options.count)]) }
    }
}

extension VerticalAlignment {
    private enum BubbleBottom: AlignmentID {
        static func defaultValue(in context: ViewDimensions) -> CGFloat { context[.bottom] }
    }

    /// 气泡的底边：群聊里的头像对齐它，不对齐气泡下面的选项按钮（ADR-0109 第 2 节）。
    static let bubbleBottom = VerticalAlignment(BubbleBottom.self)
}

/// 气泡和挂在它正下方的东西（选项按钮）：第一个子视图是气泡，其余的和它同宽，依次排在下面。
///
/// 气泡最宽到对面留出 `spacing.bubbleOpposite`（从屏幕边算起），短消息按内容收缩（ADR-0109 第 9 节）。上限在布局里算，
/// 不从外面量好了传进来：量好再传要排两遍，第二遍行高一变，进房间摆好的落点就偏了。
struct BubbleColumn: Layout {
    var spacing: CGFloat = WontonTheme.spacing.s4

    /// 这一行两边已经各让出 `spacing.bubbleEdge`，对面还要再留这么多。
    private static let reserve = WontonTheme.spacing.bubbleOpposite - WontonTheme.spacing.bubbleEdge * 2

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let bubble = subviews.first else { return .zero }
        let size = bubble.sizeThatFits(ProposedViewSize(
            width: proposal.width.map { max(0, $0 - Self.reserve) }, height: proposal.height
        ))
        let below = subviews.dropFirst().reduce(CGFloat.zero) { height, view in
            height + spacing + view.sizeThatFits(ProposedViewSize(width: size.width, height: nil)).height
        }
        return CGSize(width: size.width, height: size.height + below)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var top = bounds.minY
        for (index, view) in subviews.enumerated() {
            if index > 0 { top += spacing }
            let fitted = ProposedViewSize(width: bounds.width, height: nil)
            let height = view.sizeThatFits(fitted).height
            view.place(
                at: CGPoint(x: bounds.minX, y: top),
                proposal: ProposedViewSize(width: bounds.width, height: height)
            )
            top += height
        }
    }

    func explicitAlignment(
        of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? {
        guard guide == .bubbleBottom, let bubble = subviews.first else { return nil }
        return bounds.minY + bubble.sizeThatFits(ProposedViewSize(width: bounds.width, height: nil)).height
    }
}
