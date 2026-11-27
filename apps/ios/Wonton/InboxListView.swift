import SwiftUI
import WontonClient

/// 收件箱（ADR-0119）：要你决定和要你知道的事。确认卡片是其中一类，点开能直接回答；联系人请求排在最前。
/// 顺序是核心排好的（等着的在前，已处理的在后）。取不到时上次的还在，照实提示并给重试。
struct InboxListView: View {
    let view: InboxViewModel
    let bots: [BotView]
    let plugins: PluginsModel
    let answer: (ConfirmationView, CardChoice, [String: String], [String: SettingValue], [String]) -> Void
    let stopWindow: (ConfirmationView) -> Void
    let retry: () -> Void
    let approveContact: (PendingContactRequest) -> Void
    let rejectContact: (PendingContactRequest) -> Void
    var contactFailure: Problem?
    var contactBusy = false

    private var isEmpty: Bool { view.entries.isEmpty && view.contactRequests.isEmpty }
    private var title: String { String(localized: .Confirmations.confirmationsInboxTitle) }

    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let problem = view.problem, isEmpty {
            ProblemView(problem: problem.problem, retry: retry)
                .wontonPushedScreen(title)
        } else if view.loading && isEmpty {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .wontonPushedScreen(title)
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
            Text(.Confirmations.confirmationsInboxEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityIdentifier("inbox-empty")
        .wontonPushedScreen(title)
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
                ForEach(Array(view.entries.enumerated()), id: \.element.item.id) { index, entry in
                    if index > 0 {
                        WontonSeparator().padding(.leading, WontonTheme.size.avatar + WontonTheme.spacing.s12)
                    }
                    if let card = entry.card {
                        ConfirmationCardRow(
                            view: card,
                            bots: bots,
                            plugins: plugins,
                            answer: { answer(card, $0, $1, $2, $3) },
                            stopWindow: { stopWindow(card) }
                        )
                    } else {
                        InboxSummaryRow(item: entry.item)
                    }
                }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .wontonPushedScreen(title)
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

/// 收件箱里只有摘要的一条：发送方、标题和纯文本正文（核心给旧 App、通知和读屏的兜底）。
private struct InboxSummaryRow: View {
    let item: InboxListItem
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(item.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(item.status == .pending ? colors.textPrimary : colors.textSecondary))
                Text(item.sender.displayName)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                if !item.body.isEmpty {
                    Text(item.body)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if item.unread {
                Circle().fill(Color(colors.primary)).frame(width: 8, height: 8)
                    .padding(.top, WontonTheme.spacing.s8)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s12)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("inbox-item-\(item.id)")
    }
}
