import SwiftUI
import WontonClient

/// 有消息解不开时的一行说明和入口：账号设过备份就输恢复密钥，没设过就请另一台设备确认，整行点去备份页（ADR-0046）。
/// 这行常驻输入框上方，收成一行，别把能看到的消息挤走。
struct KeyNoticeBanner: View {
    let notice: KeyNotice
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch notice {
        case .enterRecoveryKey, .verifyWithOtherDevice:
            NavigationLink(value: ChatRoute.chatBackup) { row }
                .buttonStyle(.plain)
        // 更新的核心给出的类别没有按钮，说明照常显示。
        case .other:
            row
        }
    }

    private var row: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.lockKey.view(size: WontonTheme.size.iconTiny)
                .foregroundStyle(Color(colors.textSecondary))
            Text(notice.text)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .wontonText(WontonTheme.typography.body)
        .foregroundStyle(Color(colors.textPrimary))
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
        .frame(minHeight: WontonTheme.size.target)
    }
}

extension KeyNotice {
    /// 按类别说明密钥怎么找回；两条路的入口都在备份页（ADR-0046）。
    var text: String {
        switch self {
        case .enterRecoveryKey: String(localized: .Message1.messageKeyNoticeRecovery)
        case .verifyWithOtherDevice: String(localized: .Message1.messageKeyNoticeVerify)
        case .other: String(localized: .Message2.messageUndecryptableNotice)
        }
    }
}

/// 群里能用的 AI：在场的 Bot 一行小胶囊，点了开 Bot 资料；确定一个都没有时说明 @ 不会有人回应。
/// 私聊不显示：Bot 本来就在顶栏（ADR-0044）。成员名单没取全（roomBots 是 nil）时在场的算不准，
/// 不下“群里没有”的结论，什么都不显示。
struct RoomAiLine: View {
    let roomBots: [String]?
    let bots: [BotView]
    let core: ClientCore
    let plugins: PluginsModel
    let onOpen: (BotRef) -> Void

    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let roomBots {
            let inRoom = roomBots.compactMap { id in bots.first { $0.userId == id } }
            if inRoom.isEmpty {
                HStack(spacing: WontonTheme.spacing.s8) {
                    WontonIcon.info.view(size: WontonTheme.size.iconTiny)
                    Text(.Chat.roomNoAi)
                }
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s4)
            } else {
                HStack(spacing: WontonTheme.spacing.s8) {
                    ForEach(inRoom, id: \.userId) { bot in
                        Button {
                            onOpen(bot.bot)
                        } label: {
                            HStack(spacing: WontonTheme.spacing.s4) {
                                BotAvatarView(
                                    bot: bot,
                                    pluginIcon: pluginIcon(bot),
                                    core: core,
                                    model: plugins,
                                    size: WontonTheme.size.icon
                                )
                                Text(bot.name)
                                    .wontonText(WontonTheme.typography.caption)
                                    .foregroundStyle(Color(colors.textPrimary))
                                    .lineLimit(1)
                            }
                            .frame(minHeight: WontonTheme.size.target)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s4)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("roomAiLine")
            }
        }
    }

    private func pluginIcon(_ bot: BotView) -> PluginIcon? {
        plugins.viewModel.installed?.plugins.first { $0.id == bot.bot.pluginId }?.icon
    }
}

/// 主对话和旁聊页的顶栏（ADR-0093 §6）：主对话是 `ChatRoomBar`（返回、标题胶囊、房间入口和头像）；旁聊页的标题是
/// 旁聊的标题，还没到就先叫“旁聊”，不写状态行、没有头像和按钮。两页的背景都是聊天背景。
struct RoomTopBar<Trailing: View>: ViewModifier {
    let title: String
    let avatar: AnyView?
    /// 有人正在输入时状态行写的字（ADR-0109 第 8 节）；没人输入为空。
    var typing: String?
    /// 点头像去的资料页；没有的房间为空。
    var openProfile: (() -> Void)?
    /// 旁聊页的标题；主对话为空。
    let sideChatTitle: String?
    /// 旁聊页返回键的动作：收起旁聊，不动导航栈。
    let close: (() -> Void)?
    @ViewBuilder let trailing: () -> Trailing

    func body(content: Content) -> some View {
        content.wontonTopBar {
            if let sideChatTitle {
                WontonPushedBar(title: sideChatTitle, showsStatus: true, onClose: close) { EmptyView() }
                    .background(.regularMaterial, ignoresSafeAreaEdges: .top)
            } else {
                // 不垫底：返回、标题、头像各是一块玻璃，消息从它们下面滚过去（ADR-0107）。
                ChatRoomBar(title: title, avatar: avatar, typing: typing, openProfile: openProfile, trailing: trailing)
                    .background { ChatEdgeFade(edge: .top).ignoresSafeArea(edges: .top) }
            }
        }
        .toolbar(.hidden, for: .navigationBar)
        .background(ChatWallpaper())
    }
}
