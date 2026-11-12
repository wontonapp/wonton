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
            Text(.Contacts1.contactsEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.Contacts0.contactsAdd, action: add)
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
            Text(.Contacts1.contactsIncomingWarning)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.warning))
            HStack(spacing: WontonTheme.spacing.s8) {
                Spacer(minLength: 0)
                Button(.Contacts1.contactsReject, action: reject)
                    .buttonStyle(WontonSmallButtonStyle(kind: .danger))
                Button(.Contacts1.contactsApprove, action: approve)
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
            WontonIconButton(icon: .trash, label: String(localized: .Contacts0.contactsRemove),
                             tint: Color(colors.danger), action: remove)
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
        let action: LocalizedStringResource = waiting ? .Contacts1.contactsWithdraw : .Contacts1.contactsClear
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
                    Text(.Contacts0.contactsKeeper(keeper))
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
    case .pending: contact.kind == .person
        ? .Contacts0.contactsStatusWaitingPerson : .Contacts0.contactsStatusWaitingBot
    case .declined: .Contacts1.contactsStatusDeclined
    case .notFound: .Contacts1.contactsStatusNotFound
    case .incoming, .active, .other: .Contacts0.contactsStatusUnknown
    }
}

/// 跨机的联系人消息过不去的原因，按那台服务器的配对状态写；同机的、连得上的没有。
func contactRemoteProblem(_ contact: BotContact) -> LocalizedStringResource? {
    guard let remote = contact.remote else { return nil }
    switch remote.peerStatus {
    case nil: return .Contacts0.contactsRemoteUnpaired
    case .offline: return .Peers2.peersStatusOffline
    case .removedByPeer: return .Peers1.peersStatusRemoved
    case .publicAccessOff: return .Peers3.peersStatusPublicAccessOff
    case .connected, .checking, .awaitingApproval, .declined, .other: return nil
    }
}

/// 联系人接口失败的一句话，按核心给的类别写（ADR-0043）；核心写好的说明收进“详情”。不认识的类别用通用的一句。
func contactsProblem(_ error: ContactError, message: String) -> Problem {
    let title: LocalizedStringResource = switch error {
    case .invalidUserId: .Contacts0.contactsErrorInvalidUserId
    case .notPaired: .Contacts0.contactsErrorNotPaired
    case .alreadyContact: .Contacts1.contactsErrorAlreadyContact
    case .botNotFound: .Contacts0.contactsErrorBotNotFound
    case .contactNotFound: .Contacts0.contactsErrorContactNotFound
    case .notKeeper: .Contacts1.contactsErrorNotKeeper
    case .peerUnreachable: .Contacts0.contactsErrorPeerUnreachable
    case .unauthenticated: .Contacts1.contactsErrorUnauthenticated
    case .upstream: .Contacts1.contactsErrorUpstream
    case .transportUnavailable: .Problem2.problemTransportUnavailableTitle
    case .protocol, .other: .Peers1.peersErrorOther
    }
    let next: String? = error == .notPaired ? String(localized: .Contacts0.contactsErrorNotPairedNext) : nil
    return Problem(title: String(localized: title), nextStep: next, detail: message)
}
