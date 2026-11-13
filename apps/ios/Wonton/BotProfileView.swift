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
                DetailSection(title: String(localized: .BotProfile0.botProfileIdentity)) {
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                        Text(botRef.pluginId).accessibilityIdentifier("botProfilePluginId")
                        Text(botRef.bot).accessibilityIdentifier("botProfileBotId")
                        if let bot {
                            Text(bot.userId).accessibilityIdentifier("botProfileUserId")
                        }
                    }
                    .wontonText(WontonTheme.typography.supporting)
                    .textSelection(.enabled)
                }
                if botRef.bot == "default" {
                    BotProfileSection(chat: chat, plugins: plugins, pluginId: botRef.pluginId)
                } else if let bot {
                    readOnlyProfile(bot)
                }
                BotWebSection(botRef: botRef, bot: bot, willOpenWeb: willOpenWeb)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(bot?.name ?? String(localized: .BotProfile0.botProfileTitle), showsStatusLine: true)
        .accessibilityIdentifier("botProfile")
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

private struct BotWebSection: View {
    let botRef: BotRef
    let bot: BotView?
    let willOpenWeb: () -> Void
    @Environment(PagesModel.self) private var pages
    @Environment(\.wontonColors) private var colors

    private var page: BotWebPage? {
        guard let bot, bot.bot == botRef, let web = bot.web,
              case let .ready(page) = web, page.bot == botRef else { return nil }
        return page
    }

    var body: some View {
        DetailSection(title: String(localized: .BotProfile1.botProfileWebTitle)) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                if let page {
                    Text(page.url)
                        .wontonText(WontonTheme.typography.supporting)
                        .textSelection(.enabled)
                        .accessibilityIdentifier("botProfileWebAddress")
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
