import SwiftUI
import WontonClient

/// The identity is captured from the chat's BotView, never reconstructed from its Matrix user ID.
struct BotProfileView: View {
    let chat: ChatModel
    let plugins: PluginsModel
    let botRef: BotRef
    var willOpenWeb: () -> Void = {}
    @Environment(\.wontonColors) private var colors

    private var bot: BotView? { chat.viewModel.bots.first { $0.bot == botRef } }
    private var plugin: InstalledPlugin? {
        plugins.viewModel.installed?.plugins.first { $0.id == botRef.pluginId }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if botRef.bot == "default" {
                    BotProfileSection(chat: chat, plugins: plugins, pluginId: botRef.pluginId)
                } else if let bot {
                    readOnlyProfile(bot)
                } else {
                    Text(.BotProfile1.botProfileMissing)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .accessibilityIdentifier("botProfileUnavailable")
                }
                if bot != nil {
                    BotWebAction(botRef: botRef, bot: bot, willOpenWeb: willOpenWeb)
                }
                details
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(bot?.name ?? String(localized: .BotProfile0.botProfileTitle), showsStatusLine: true)
        .accessibilityIdentifier("botProfile")
    }

    /// 插件、账号和网页地址这些技术标识，默认收起。
    private var details: some View {
        ProblemDetail(
            text: detailsText,
            minimumTargetHeight: WontonTheme.size.target,
            toggleIdentifier: "botProfileDetails"
        )
    }

    private var detailsText: String {
        var lines = [String(localized: .BotProfile1.botProfileDetailsIdentity(botRef.pluginId, botRef.bot))]
        if let bot {
            lines.append(String(localized: .BotProfile1.botProfileDetailsAccount(bot.userId)))
        }
        if let url = bot?.webPage?.url {
            lines.append(String(localized: .BotProfile0.botProfileDetailsWeb(url)))
        }
        return lines.joined(separator: "\n")
    }

    private func readOnlyProfile(_ bot: BotView) -> some View {
        DetailSection(title: String(localized: .BotProfile0.botProfileTitle)) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                HStack(spacing: WontonTheme.spacing.s16) {
                    BotAvatarView(
                        bot: bot, pluginIcon: plugin?.icon, core: chat.core,
                        model: plugins, size: WontonTheme.size.detailIcon
                    )
                    Text(bot.name)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .accessibilityIdentifier("botProfileName")
                }
                Text(.BotProfile0.botProfileReadOnly)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .accessibilityIdentifier("botProfileReadOnly")
            }
        }
    }
}

/// 只有核心给了这个 Bot 的网页地址时才给入口（家人要主人开放，由核心判断）；不能打开的一句话就地说明。
/// 按钮文字写明打开的是什么，网页里的界面是英文。
private struct BotWebAction: View {
    let botRef: BotRef
    let bot: BotView?
    let willOpenWeb: () -> Void
    @Environment(PagesModel.self) private var pages
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if bot?.webPage != nil {
            Button(String(localized: .BotProfile0.botProfileWebOpen)) {
                willOpenWeb()
                pages.open(.bot(bot: botRef, path: nil))
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
            .accessibilityIdentifier("botProfileWebOpen")
        } else {
            Text(unavailableText)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .accessibilityIdentifier("botProfileWebUnavailable")
        }
    }

    private var unavailableText: String {
        guard let bot, bot.bot == botRef else { return String(localized: .BotProfile1.botProfileWebUnknown) }
        switch bot.web {
        case let .unavailable(reason): return reason.profileText
        case .ready, .other, nil: return String(localized: .BotProfile1.botProfileWebUnknown)
        }
    }
}

extension BotWebFailure {
    var profileText: String {
        switch self {
        case .unauthenticated: String(localized: .BotProfile0.botProfileWebUnauthenticated)
        case .notShared: String(localized: .BotProfile0.botProfileWebNotShared)
        case .permissionDenied: String(localized: .BotProfile1.botProfileWebPermissionDenied)
        case .notFound: String(localized: .BotProfile1.botProfileWebNotFound)
        case .noWebui: String(localized: .BotProfile0.botProfileWebNoWebui)
        case .notReady: String(localized: .BotProfile0.botProfileWebNotReady)
        case .other: String(localized: .BotProfile1.botProfileWebUnknown)
        }
    }
}
