import SwiftUI
import WontonClient

/// 还没有好友：一句话和加好友。
struct FriendsEmpty: View {
    let add: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text(.friendsEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.friendsAdd, action: add)
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
            if request.kind == .friend {
                Label {
                    Text(.friendsFriendBadge)
                } icon: {
                    WontonIcon.person.view(size: WontonTheme.size.iconSmall)
                }
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.primary))
            }
            Text(.friendsReceived(relativeFriendTime(request.invitedAtMs)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(.friendsAccept, action: accept)
                    .buttonStyle(WontonSmallButtonStyle())
                Button(.friendsDecline, action: decline)
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
                Button(.friendsCancel, action: cancel)
                    .buttonStyle(WontonSmallButtonStyle())
            }
            Text(.friendsOutgoingWaiting)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            if request.kind == .friend {
                Text(.friendsSent(relativeFriendTime(request.invitedAtMs)))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }
}

/// 一个好友：名字（没有就用户 ID）、所在分组、分享给他的插件数。
struct FriendRowView: View {
    let row: FriendRow
    let groupNames: [String]
    let remove: () -> Void
    let chooseGroups: () -> Void
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
                Button(.friendsGroups, action: chooseGroups)
                    .buttonStyle(WontonSmallButtonStyle())
                Button(action: block) {
                    WontonIcon.blocked.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .buttonStyle(WontonSmallButtonStyle())
                .accessibilityLabel(String(localized: .friendsBlock))
                Button(role: .destructive, action: remove) {
                    WontonIcon.trash.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.danger))
                }
                .buttonStyle(WontonSmallButtonStyle(kind: .danger))
                .accessibilityLabel(String(localized: .friendsRemove))
            }
            if groupNames.isEmpty {
                Text(.friendsGroupsEmpty)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                Text(groupNames.joined(separator: String(localized: .friendsGroupsJoin)))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            Text(row.sharedPlugins.isEmpty
                 ? String(localized: .friendsSharedNone)
                 : String(localized: .friendsShared(row.sharedPlugins.count)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }
}

/// 一个分组：名字，改名在行尾，删除在左滑。
struct FriendGroupRowView: View {
    let group: FriendGroup
    let rename: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            Text(group.name)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .layoutPriority(1)
            Spacer(minLength: 0)
            Button(.friendsGroupsRename, action: rename)
                .buttonStyle(WontonSmallButtonStyle())
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
            Button(.friendsUndo, action: unblock)
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
    case .other: String(localized: .friendsBlockUnknown)
    }
}

private func relativeFriendTime(_ millis: UInt64) -> String {
    Date(timeIntervalSince1970: TimeInterval(millis) / 1000).formatted(.relative(presentation: .named))
}
