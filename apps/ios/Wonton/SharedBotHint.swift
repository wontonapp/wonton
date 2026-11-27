import SwiftUI
import WontonClient

/// 共享 Bot 的长期记忆全家共用：第一次和它的私聊顶部提示一次，关了就按房间记住。
struct SharedBotMemoryHint: View {
    let dismiss: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            WontonIcon.info.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            Text(.Family.familySharedBotMemory)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textPrimary))
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: dismiss) {
                WontonIcon.close.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
                    .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(String(localized: .Family.familyDismiss))
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
    }
}

/// 记哪些共享 Bot 私聊已经提示过；键是房间加 Bot 的 Matrix ID。
struct SharedBotHintSeen {
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// 已经关掉的提示键；页面首帧直接读这份。
    var dismissedKeys: Set<String> { dismissed() }

    func seen(roomId: String, botUserId: String) -> Bool {
        dismissed().contains(key(roomId, botUserId))
    }

    func dismiss(roomId: String, botUserId: String) {
        var keys = dismissed()
        keys.insert(key(roomId, botUserId))
        defaults.set(keys.sorted(), forKey: Self.storageKey)
    }

    private func dismissed() -> Set<String> {
        Set(defaults.stringArray(forKey: Self.storageKey) ?? [])
    }

    private func key(_ roomId: String, _ botUserId: String) -> String { "\(roomId)|\(botUserId)" }

    private static let storageKey = "sharedBotMemoryHintSeen"
}
