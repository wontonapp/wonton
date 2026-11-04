import SwiftUI
import WontonClient

/// 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。
struct ConfirmationCardView: View {
    let view: ConfirmationView
    let answer: (Answer) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(view.card.title).wontonText(WontonTheme.typography.itemTitle)
            ForEach(Array(view.card.lines.enumerated()), id: \.offset) { _, line in
                Text(line.text)
                    .wontonText(WontonTheme.typography.supporting)
                    .fontWeight(line.emphasis ? .semibold : .regular)
                    .foregroundStyle(line.emphasis ? Color(colors.danger) : Color(colors.textPrimary))
            }
            state
            if let problem = view.problem {
                note(problem.message, icon: .warning, color: colors.danger)
            }
            if !view.card.choices.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: WontonTheme.spacing.s8) { buttons }
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) { buttons }
                }
            }
        }
        // 时间线第一次量这一行时给的高度可能不够，主人没读完不该按确认。
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder private var state: some View {
        switch view.card.state {
        case .pending:
            EmptyView()
        case .running:
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text("正在执行").wontonText(WontonTheme.typography.supporting)
            }
        case let .succeeded(message):
            note(message, icon: .checkCircle, color: colors.success)
        case let .failed(message):
            note("没有成功：\(message)", icon: .error, color: colors.danger)
        case let .uncertain(message):
            note("结果不确定：\(message)", icon: .help, color: colors.warning)
        case .rejected:
            Text("已拒绝，没有执行")
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case .expired:
            Text("已过期，没有执行")
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
    }

    private func note(_ text: String, icon: WontonIcon, color: Rgb) -> some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            icon.view(size: WontonTheme.size.iconSmall)
            Text(text).wontonText(WontonTheme.typography.supporting)
        }
        .foregroundStyle(Color(color))
    }

    private var buttons: some View {
        ForEach(view.card.choices, id: \.answer) { choice in
            Button {
                answer(choice.answer)
            } label: {
                HStack(spacing: WontonTheme.spacing.s8) {
                    if view.sending == choice.answer { ProgressView() }
                    Text(choice.label)
                }
            }
            .buttonStyle(WontonButtonStyle(kind: choice.answer == .approve ? .primary : .secondary))
            .disabled(view.sending != nil)
        }
    }
}
