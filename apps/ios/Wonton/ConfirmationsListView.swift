import SwiftUI
import WontonClient

/// “等你确认”列表（ADR-0016）：代替原来 Wonton 私聊里的卡片记录。等回答的在前、结束 24 小时内的在后，
/// 每张能点开回答。取不到列表时上次的还在，照实提示并给重试。
struct ConfirmationsListView: View {
    let view: ConfirmationsViewModel
    let bots: [BotView]
    let plugins: PluginsModel
    let answer: (ConfirmationView, CardChoice, [String: String], [String: SettingValue], [String]) -> Void
    let stopWindow: (ConfirmationView) -> Void
    let retry: () -> Void
    let approveContact: (PendingContactRequest) -> Void
    let rejectContact: (PendingContactRequest) -> Void
    var contactFailure: Problem?
    var contactBusy = false

    private var isEmpty: Bool { view.pending.isEmpty && view.finished.isEmpty && view.contactRequests.isEmpty }

    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let problem = view.problem, isEmpty {
            ProblemView(problem: problem.problem, retry: retry)
                .wontonPushedScreen(String(localized: .Confirmations.confirmationsTitle))
        } else if view.loading && isEmpty {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .wontonPushedScreen(String(localized: .Confirmations.confirmationsTitle))
        } else if isEmpty && contactFailure == nil {
            empty
        } else {
            list
        }
    }

    private var empty: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.checkCircle.view()
                .foregroundStyle(Color(colors.textSecondary))
            Text(.Confirmations.confirmationsEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .wontonPushedScreen(String(localized: .Confirmations.confirmationsTitle))
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
                if view.loading { ProgressView() }
                if let contactFailure {
                    ProblemRow(problem: contactFailure)
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                }
                ForEach(Array(view.contactRequests.enumerated()), id: \.offset) { _, request in
                    contactRow(request)
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
                        answer: { answer(card, $0, $1, $2, $3) },
                        stopWindow: { stopWindow(card) }
                    )
                }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .wontonPushedScreen(String(localized: .Confirmations.confirmationsTitle))
    }

    private func contactRow(_ request: PendingContactRequest) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            let bot = bots.first { $0.bot == request.bot }
            Text(bot?.name ?? request.bot.bot)
                .wontonText(WontonTheme.typography.itemTitle)
            Text(request.bot.pluginId)
                .wontonText(WontonTheme.typography.supporting)
            IncomingContactRowView(contact: request.contact,
                approve: { approveContact(request) }, reject: { rejectContact(request) })
                .disabled(contactBusy)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("confirmation-contact-\(request.contact.userId)")
    }
}
