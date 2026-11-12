import SwiftUI
import WontonClient

/// 确认卡片行（ADR-0016）：会话输入栏上方和“等你确认”列表共用。收起是一行——发起者头像、标题、
/// 到期时间或结果；点开是完整卡片（说明行、按钮、密钥和设置表单、放行时段），能直接回答。
struct ConfirmationCardRow: View {
    let view: ConfirmationView
    let bots: [BotView]
    let plugins: PluginsModel
    let answer: (CardChoice, [String: String]) -> Void
    let stopWindow: () -> Void

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Button {
                expanded.toggle()
            } label: {
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
            .buttonStyle(.plain)
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

/// 会话里输入栏上方的一行：显示最早到期的一张，多张时带“还有 N 张”，点开第一张的完整卡片。
struct ConfirmationCardBar: View {
    let views: [ConfirmationView]
    let bots: [BotView]
    let plugins: PluginsModel
    let answer: (CardChoice, [String: String]) -> Void
    let stopWindow: () -> Void

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Button {
                expanded.toggle()
            } label: {
                ConfirmationCardHeader(
                    title: first.card.title,
                    requester: first.card.requester,
                    bots: bots,
                    plugins: plugins,
                    secondary: barSecondary,
                    trailing: views.count > 1 ? nil : CardRowLine.text(of: first),
                    expanded: expanded
                )
            }
            .buttonStyle(.plain)
            if expanded {
                ConfirmationCardView(view: first, answer: answer, stopWindow: stopWindow)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var first: ConfirmationView { views[0] }

    /// 多张时显示“还有 N 张”；xcstrings 复数符号的参数是 Int32。
    private var barSecondary: String? {
        let more = views.count - 1
        return more > 0 ? String(localized: .confirmationsMoreCards(Int32(more))) : nil
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
enum CardRowLine {
    /// 谁在哪台设备上拒绝了；没记回答者时为空。
    static func rejected(of view: ConfirmationView) -> String? {
        answered(view.card) { device, who in
            String(localized: .confirmationsRejectedByDevice(device, who))
        } plain: { who in
            String(localized: .confirmationsRejectedBy(who))
        }
    }

    /// 谁在哪台设备上允许了；没记回答者时为空。
    static func approved(of view: ConfirmationView) -> String? {
        answered(view.card) { device, who in
            String(localized: .confirmationsApprovedByDevice(device, who))
        } plain: { who in
            String(localized: .confirmationsApprovedBy(who))
        }
    }

    /// 有回答者时拼一句：有设备名带设备，没有时只说人。
    private static func answered(
        _ card: ConfirmationCard,
        both: (String, String) -> String,
        plain: (String) -> String
    ) -> String? {
        guard let who = card.answeredBy else { return nil }
        guard let device = card.answeredByDevice else { return plain(who) }
        return both(device, who)
    }
    static func text(of view: ConfirmationView) -> String? {
        switch view.card.state {
        case .pending:
            guard view.card.expiresAtMs > 0 else { return nil }
            let date = Date(timeIntervalSince1970: TimeInterval(view.card.expiresAtMs) / 1000)
            return String(localized: .confirmationsUntil(date.formatted(.dateTime.hour().minute())))
        case .running:
            return String(localized: .confirmationCardRunning)
        case .succeeded:
            return CardRowLine.approved(of: view) ?? String(localized: .confirmationsStateSucceeded)
        case let .failed(message):
            if message.isEmpty { return String(localized: .confirmationsStateFailed) }
            return String(localized: .confirmationCardFailedResult(message))
        case let .uncertain(message):
            if message.isEmpty { return String(localized: .confirmationsStateUncertain) }
            return String(localized: .confirmationCardUncertainResult(message))
        case .rejected:
            return CardRowLine.rejected(of: view) ?? String(localized: .confirmationCardRejected)
        case .expired:
            return String(localized: .confirmationCardExpired)
        }
    }
}
