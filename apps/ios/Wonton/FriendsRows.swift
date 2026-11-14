import SwiftUI
import WontonClient

/// 还没有好友：一句话和加好友。
struct FriendsEmpty: View {
    let add: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text(.Friends1.friendsEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.Friends1.friendsAdd, action: add)
                .buttonStyle(WontonButtonStyle())
        }
        .frame(maxWidth: .infinity)
        .padding(.top, WontonTheme.spacing.s48)
    }
}

/// 一条收到的好友请求：名字或用户 ID、收到的时刻、同意和拒绝。普通私聊邀请也在请求里，只是不标“好友请求”。
struct FriendRequestRowView: View {
    let request: FriendRequest
    let accept: () -> Void
    let decline: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s12) {
                name
                Spacer(minLength: 0)
            }
            if let badge = friendRequestBadge(request.kind) {
                Label {
                    Text(badge)
                } icon: {
                    WontonIcon.person.view(size: WontonTheme.size.iconSmall)
                }
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.primary))
            }
            Text(.Friends3.friendsReceived(relativeFriendTime(request.invitedAtMs)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(.Friends2.friendsAccept, action: accept)
                    .buttonStyle(WontonSmallButtonStyle())
                Button(.Friends0.friendsDecline, action: decline)
                    .buttonStyle(WontonSmallButtonStyle(kind: .danger))
            }
            .padding(.top, WontonTheme.spacing.s4)
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }

    private var name: some View {
        Text(friendName(request.displayName, userId: request.userId))
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textPrimary))
            .layoutPriority(1)
    }
}

/// 邀请上的标签：好友请求、别人家的 Bot 请你做联系人（ADR-0050，接受就是同意）；普通私聊邀请和不认识的种类不加。
func friendRequestBadge(_ kind: FriendRequestKind) -> LocalizedStringResource? {
    switch kind {
    case .friend: .Friends0.friendsFriendBadge
    case .botContact: .Friends0.friendsBotContactBadge
    case .directChat, .other: nil
    }
}

/// 一条自己发出、对方还没处理的请求：等对方同意，能取消。
struct OutgoingFriendRequestRowView: View {
    let request: FriendRequest
    let cancel: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(friendName(request.displayName, userId: request.userId))
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .layoutPriority(1)
                Spacer(minLength: 0)
                Button(.Friends0.friendsCancel, action: cancel)
                    .buttonStyle(WontonSmallButtonStyle())
            }
            Text(.Friends0.friendsOutgoingWaiting)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            if request.kind == .friend {
                Text(.Friends0.friendsSent(relativeFriendTime(request.invitedAtMs)))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }
}

/// 一个好友：名字（没有就用户 ID）、分享给他的插件数。
struct FriendRowView: View {
    let row: FriendRow
    let remove: () -> Void
    let block: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s8) {
                Text(friendName(row.displayName, userId: row.userId))
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .layoutPriority(1)
                Spacer(minLength: 0)
                Button(action: block) {
                    WontonIcon.blocked.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .buttonStyle(WontonSmallButtonStyle())
                .accessibilityLabel(String(localized: .Friends3.friendsBlock))
                Button(role: .destructive, action: remove) {
                    WontonIcon.trash.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.danger))
                }
                .buttonStyle(WontonSmallButtonStyle(kind: .danger))
                .accessibilityLabel(String(localized: .Friends2.friendsRemove))
            }
            Text(row.sharedPlugins.isEmpty
                 ? String(localized: .Friends1.friendsSharedNone)
                 : String(localized: .Friends1.friendsShared(row.sharedPlugins.count)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }
}

/// 一条屏蔽：被屏蔽的人或服务器，能撤销。
struct BlockedFriendRowView: View {
    let block: FriendBlock
    let unblock: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            WontonIcon.blocked.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            Text(friendBlockName(block.target))
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .layoutPriority(1)
            Spacer(minLength: 0)
            Button(.Friends3.friendsUndo, action: unblock)
                .buttonStyle(WontonSmallButtonStyle())
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }
}

/// 好友的显示名；没填资料时是完整的 Matrix 用户 ID。
func friendName(_ displayName: String?, userId: String) -> String {
    guard let displayName, !displayName.isEmpty else { return userId }
    return displayName
}

/// 屏蔽目标的名字：服务器名或用户 ID。
func friendBlockName(_ target: FriendBlockTarget) -> String {
    switch target {
    case let .user(userId): userId
    case let .server(serverName): serverName
    case .other: String(localized: .Friends2.friendsBlockUnknown)
    }
}

private func relativeFriendTime(_ millis: UInt64) -> String {
    Date(timeIntervalSince1970: TimeInterval(millis) / 1000).formatted(.relative(presentation: .named))
}
