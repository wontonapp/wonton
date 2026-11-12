import SwiftUI
import WontonClient

/// 显示名；没有时用 Matrix 用户 ID。
func contactName(_ contact: BotContact) -> String {
    if let name = contact.displayName, !name.trimmingCharacters(in: .whitespaces).isEmpty { return name }
    return contact.userId
}

/// 还没有联系人：一句话和添加。
struct BotContactsEmpty: View {
    let add: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text(.contactsEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.contactsAdd, action: add)
                .buttonStyle(WontonButtonStyle())
        }
        .frame(maxWidth: .infinity)
        .padding(.top, WontonTheme.spacing.s48)
    }
}

/// 发来的请求：说清同意意味着什么，同意或拒绝。
struct IncomingContactRowView: View {
    let contact: BotContact
    let approve: () -> Void
    let reject: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ContactRowFrame(contact: contact) {
            Text(.contactsIncomingWarning)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.warning))
            HStack(spacing: WontonTheme.spacing.s8) {
                Spacer(minLength: 0)
                Button(.contactsReject, action: reject)
                    .buttonStyle(WontonSmallButtonStyle(kind: .danger))
                Button(.contactsApprove, action: approve)
                    .buttonStyle(WontonSmallButtonStyle())
            }
            .padding(.top, WontonTheme.spacing.s4)
        } trailing: {
            EmptyView()
        }
    }
}

/// 双方都同意了的联系人：所在的节已经说明生效，行上不再写状态；跨机的说明消息为什么过不去。
struct ActiveContactRowView: View {
    let contact: BotContact
    let remove: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ContactRowFrame(contact: contact) {
            EmptyView()
        } trailing: {
            WontonIconButton(icon: .trash, label: String(localized: .contactsRemove), tint: Color(colors.danger),
                             action: remove)
        }
    }
}

/// 自己发出、还没生效的：等对方、对方拒绝了、没有这个人。等对方的可撤销，其余的可删掉这一行。
struct SentContactRowView: View {
    let contact: BotContact
    let remove: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let waiting = contact.state == .pending
        let action: LocalizedStringResource = waiting ? .contactsWithdraw : .contactsClear
        ContactRowFrame(contact: contact) {
            Text(contactStatus(contact))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        } trailing: {
            Button(action, action: remove)
                .buttonStyle(WontonSmallButtonStyle(kind: waiting ? .secondary : .danger))
        }
    }
}

/// 一行的骨架：名字、谁的 Bot、用户 ID、跨机时的连接情况，下面是各状态自己的内容，行尾是操作。
private struct ContactRowFrame<Content: View, Trailing: View>: View {
    let contact: BotContact
    @ViewBuilder let content: Content
    @ViewBuilder let trailing: Trailing
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(contactName(contact))
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if let keeper = contact.keeperDisplayName, !keeper.isEmpty {
                    Text(.contactsKeeper(keeper))
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                Text(verbatim: contact.userId)
                    .wontonText(WontonTheme.typography.caption)
                    .monospaced()
                    .foregroundStyle(Color(colors.textSecondary))
                if let problem = contactRemoteProblem(contact) {
                    Text(problem)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.warning))
                }
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }
}

/// 还没生效的一行写什么：等对方时按对方是 Bot 还是人选“同意”或“接受”（ADR-0050）。
func contactStatus(_ contact: BotContact) -> LocalizedStringResource {
    switch contact.state {
    case .pending: contact.kind == .person ? .contactsStatusWaitingPerson : .contactsStatusWaitingBot
    case .declined: .contactsStatusDeclined
    case .notFound: .contactsStatusNotFound
    case .incoming, .active, .other: .contactsStatusUnknown
    }
}

/// 跨机的联系人消息过不去的原因，按那台服务器的配对状态写；同机的、连得上的没有。
func contactRemoteProblem(_ contact: BotContact) -> LocalizedStringResource? {
    guard let remote = contact.remote else { return nil }
    switch remote.peerStatus {
    case nil: return .contactsRemoteUnpaired
    case .offline: return .peersStatusOffline
    case .removedByPeer: return .peersStatusRemoved
    case .connected, .checking, .awaitingApproval, .declined, .other: return nil
    }
}

/// 联系人接口失败的一句话，按核心给的类别写（ADR-0043）；核心写好的说明收进“详情”。不认识的类别用通用的一句。
func contactsProblem(_ error: ContactError, message: String) -> Problem {
    let title: LocalizedStringResource = switch error {
    case .invalidUserId: .contactsErrorInvalidUserId
    case .notPaired: .contactsErrorNotPaired
    case .alreadyContact: .contactsErrorAlreadyContact
    case .botNotFound: .contactsErrorBotNotFound
    case .contactNotFound: .contactsErrorContactNotFound
    case .notKeeper: .contactsErrorNotKeeper
    case .peerUnreachable: .contactsErrorPeerUnreachable
    case .unauthenticated: .contactsErrorUnauthenticated
    case .upstream: .contactsErrorUpstream
    case .transportUnavailable: .problemTransportUnavailableTitle
    case .protocol, .other: .peersErrorOther
    }
    let next: String? = error == .notPaired ? String(localized: .contactsErrorNotPairedNext) : nil
    return Problem(title: String(localized: title), nextStep: next, detail: message)
}
