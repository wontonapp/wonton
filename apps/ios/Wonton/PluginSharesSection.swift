import SwiftUI
import WontonClient

struct PluginSharesSection: View {
    let model: PluginsModel
    let pluginId: String
    let shares: PluginShares
    let updated: (PluginDetail) -> Void
    @State private var recipients: FriendsViewModel?
    @State private var editing: ShareGrant?
    @State private var presenting = false
    @State private var problem: Problem?

    var body: some View {
        DetailSection(title: String(localized: .shareTitle)) {
            Text(.shareCount(count: Int(shares.friendCount)))
                .wontonText(WontonTheme.typography.body)
            ForEach(shares.grants, id: \.id) { grant in
                Button {
                    editing = grant
                    presenting = true
                } label: {
                    HStack {
                        Text(recipientName(grant.recipient, in: recipients))
                        Spacer()
                        if let expiry = grant.expiresAtMs {
                            Text(Date(timeIntervalSince1970: Double(expiry) / 1000), format: .dateTime)
                        } else { Text(.shareIndefinite) }
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            Button(.shareAdd) { editing = nil; presenting = true }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            if let problem {
                ProblemRow(problem: problem) { Task { await loadRecipients() } }
            }
        }
        .task(id: shares) { await loadRecipients() }
        .sheet(isPresented: $presenting) {
            PluginShareEditor(model: model, pluginId: pluginId, shares: shares, grant: editing) { detail in
                updated(detail)
                presenting = false
            }
        }
    }

    private func loadRecipients() async {
        problem = nil
        do { recipients = try await model.core.shareRecipients() } catch { problem = error.problem }
    }
}

func recipientName(_ recipient: ShareRecipient, in view: FriendsViewModel?) -> String {
    switch recipient {
    case let .friend(userId):
        return view?.friends.first { $0.userId == userId }?.displayName.flatMap { $0.isEmpty ? nil : $0 } ?? userId
    case let .group(groupId):
        return view?.groups.first { $0.id == groupId }?.name ?? groupId
    case .other: return String(localized: .shareUnknown)
    }
}
