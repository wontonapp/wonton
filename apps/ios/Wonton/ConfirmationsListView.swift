import SwiftUI
import WontonClient

/// 列表页的接线：订阅核心的列表，回答经聊天模型发出，问题经它的弹窗显示。
struct ConfirmationsPage: View {
    let chat: ChatModel
    let plugins: PluginsModel
    @State private var confirmations: ConfirmationsModel

    init(chat: ChatModel, plugins: PluginsModel) {
        self.chat = chat
        self.plugins = plugins
        _confirmations = State(initialValue: ConfirmationsModel(core: chat.core))
    }

    var body: some View {
        ConfirmationsListView(
            view: confirmations.viewModel,
            bots: chat.viewModel.bots,
            plugins: plugins,
            answer: { view, choice, secrets in
                chat.send(.answerConfirmation(
                    id: view.card.id, answer: choice.answer, choiceId: choice.choiceId,
                    secrets: secrets, settings: [:]
                ))
            },
            stopWindow: { view in chat.send(.stopConfirmationWindow(id: view.card.id)) },
            retry: { chat.send(.retryConfirmations) }
        )
        .task { await confirmations.observe() }
    }
}

/// “等你确认”列表（ADR-0016）：代替原来 Wonton 私聊里的卡片记录。等回答的在前、结束 24 小时内的在后，
/// 每张能点开回答。取不到列表时上次的还在，照实提示并给重试。
struct ConfirmationsListView: View {
    let view: ConfirmationsViewModel
    let bots: [BotView]
    let plugins: PluginsModel
    let answer: (ConfirmationView, CardChoice, [String: String]) -> Void
    let stopWindow: (ConfirmationView) -> Void
    let retry: () -> Void

    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let problem = view.problem, view.pending.isEmpty, view.finished.isEmpty {
            ProblemView(problem: problem.problem, retry: retry)
        } else if view.loading {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .wontonPushedScreen(String(localized: .confirmationsTitle))
        } else if view.pending.isEmpty, view.finished.isEmpty {
            empty
        } else {
            list
        }
    }

    private var empty: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.checkCircle.view()
                .foregroundStyle(Color(colors.textSecondary))
            Text(.confirmationsEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .wontonPushedScreen(String(localized: .confirmationsTitle))
    }

    private var list: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                if let problem = view.problem {
                    ProblemRow(problem: problem.problem, retry: retry)
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .padding(.vertical, WontonTheme.spacing.s8)
                    WontonSeparator()
                }
                ForEach(Array((view.pending + view.finished).enumerated()), id: \.element.card.id) { index, card in
                    if index > 0 {
                        WontonSeparator().padding(.leading, WontonTheme.size.avatar + WontonTheme.spacing.s12)
                    }
                    ConfirmationCardRow(
                        view: card,
                        bots: bots,
                        plugins: plugins,
                        answer: { answer(card, $0) },
                        stopWindow: { stopWindow(card) }
                    )
                }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .wontonPushedScreen(String(localized: .confirmationsTitle))
    }
}
