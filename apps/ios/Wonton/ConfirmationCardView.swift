import SwiftUI
import WontonClient

/// 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。
struct ConfirmationCardView: View {
    let view: ConfirmationView
    let answer: (Answer) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(view.card.title).font(.headline)
            ForEach(Array(view.card.lines.enumerated()), id: \.offset) { _, line in
                Text(line.text)
                    .font(.subheadline)
                    .fontWeight(line.emphasis ? .semibold : .regular)
                    .foregroundStyle(line.emphasis ? Color.red : Color.primary)
            }
            state
            if let problem = view.problem {
                Label(problem.message, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.red)
            }
            if !view.card.choices.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack { buttons }
                    VStack(alignment: .leading) { buttons }
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
            HStack(spacing: 6) {
                ProgressView()
                Text("正在执行")
            }
            .font(.footnote)
        case let .succeeded(message):
            Label(message, systemImage: "checkmark.circle").font(.footnote).foregroundStyle(.green)
        case let .failed(message):
            Label("没有成功：\(message)", systemImage: "xmark.octagon").font(.footnote).foregroundStyle(.red)
        case let .uncertain(message):
            Label("结果不确定：\(message)", systemImage: "questionmark.circle")
                .font(.footnote)
                .foregroundStyle(.orange)
        case .rejected:
            Text("已拒绝，没有执行").font(.footnote).foregroundStyle(.secondary)
        case .expired:
            Text("已过期，没有执行").font(.footnote).foregroundStyle(.secondary)
        }
    }

    private var buttons: some View {
        ForEach(view.card.choices, id: \.answer) { choice in
            if choice.answer == .approve {
                // 气泡给内容统一设了正文颜色，会盖掉主按钮自己的文字色，和按钮底色一样时字就看不见了。
                button(choice).buttonStyle(.borderedProminent).foregroundStyle(Color(colors.onPrimary))
            } else {
                button(choice).buttonStyle(.bordered)
            }
        }
    }

    private func button(_ choice: CardChoice) -> some View {
        Button {
            answer(choice.answer)
        } label: {
            HStack(spacing: 6) {
                if view.sending == choice.answer { ProgressView() }
                Text(choice.label)
            }
        }
        .disabled(view.sending != nil)
    }
}
