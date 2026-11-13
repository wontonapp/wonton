import SwiftUI
import WontonClient

/// “等你确认”列表里的确认卡片行：收起是一行——发起者头像、标题和结果；点开是完整卡片，能直接回答。
/// 完整卡片复用说明行、按钮、密钥和设置表单。
struct ConfirmationCardRow: View {
    let view: ConfirmationView
    let bots: [BotView]
    let plugins: PluginsModel
    let answer: (CardChoice, [String: String], [String: SettingValue]) -> Void
    let stopWindow: () -> Void

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Button {
                expanded.toggle()
            } label: {
                SwiftUI.TimelineView(.periodic(from: .now, by: 30)) { _ in
                ConfirmationCardHeader(
                    title: view.card.title,
                    requester: view.card.requester,
                    bots: bots,
                    plugins: plugins,
                    secondary: nil,
                    trailing: CardRowLine.text(of: view),
                    expanded: expanded
                )
                }
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("confirmation-row-\(view.card.id)")
            if expanded {
                ConfirmationCardView(view: view, answer: answer, stopWindow: stopWindow)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
        .contentShape(Rectangle())
    }
}

/// 收起的卡片行：发起者头像、标题和一行次要信息，右侧箭头指示能不能展开。
struct ConfirmationCardHeader: View {
    let title: String
    let requester: ConfirmationRequester?
    let bots: [BotView]
    let plugins: PluginsModel
    /// 标题下面的一行：“还有 N 张”这类。
    let secondary: String?
    /// 到期时间或结果。
    let trailing: String?
    let expanded: Bool

    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            ConfirmationRequesterAvatar(requester: requester, bots: bots, plugins: plugins)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .multilineTextAlignment(.leading)
                if let secondary {
                    Text(secondary)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
                if let trailing {
                    Text(trailing)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
                .rotationEffect(.degrees(expanded ? 90 : 0))
        }
    }
}

/// 按 requester 画头像：Bot 按 `user_id` 在 bots 里找（ADR-0044），插件用插件图标；都不认识时用占位。
struct ConfirmationRequesterAvatar: View {
    let requester: ConfirmationRequester?
    let bots: [BotView]
    let plugins: PluginsModel
    var size = WontonTheme.size.avatar

    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch requester {
        case let .bot(bot, userId):
            if let view = bots.first(where: { $0.userId == userId }) {
                BotAvatarView(bot: view, pluginIcon: icon(bot.pluginId), core: plugins.core, model: plugins, size: size)
            } else {
                placeholder
            }
        case let .plugin(pluginId):
            PluginIconView(icon: icon(pluginId), size: size, model: plugins)
        case .other, nil:
            placeholder
        }
    }

    private var placeholder: some View {
        WontonIcon.person.view(size: WontonTheme.size.icon)
            .foregroundStyle(Color(colors.textSecondary))
            .frame(width: size, height: size)
            .background(Circle().fill(Color(colors.surfaceSubtle)))
            .accessibilityHidden(true)
    }

    private func icon(_ pluginId: String) -> PluginIcon? {
        plugins.viewModel.installed?.plugins.first { $0.id == pluginId }?.icon
    }
}

/// 收起行上显示的一句：等回答时是到期时刻，结束的是结果。
private enum CardRowLine {
    static func text(of view: ConfirmationView) -> String? {
        switch view.card.state {
        case .pending:
            guard view.card.expiresAtMs > 0 else { return nil }
            let date = Date(timeIntervalSince1970: TimeInterval(view.card.expiresAtMs) / 1000)
            let remaining = date.formatted(.relative(presentation: .numeric, unitsStyle: .abbreviated))
            return String(localized: .Confirmations.confirmationsUntil(remaining))
        case .running:
            return String(localized: .ConfirmationCard.confirmationCardRunning)
        case .succeeded:
            return String(localized: .Confirmations.confirmationsStateSucceeded)
        case let .failed(message):
            if message.isEmpty { return String(localized: .Confirmations.confirmationsStateFailed) }
            return String(localized: .ConfirmationCard.confirmationCardFailedResult(message))
        case let .uncertain(message):
            if message.isEmpty { return String(localized: .Confirmations.confirmationsStateUncertain) }
            return String(localized: .ConfirmationCard.confirmationCardUncertainResult(message))
        case .rejected:
            return String(localized: .ConfirmationCard.confirmationCardRejected)
        case .expired:
            return String(localized: .ConfirmationCard.confirmationCardExpired)
        }
    }
}
