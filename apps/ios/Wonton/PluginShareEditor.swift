import SwiftUI
import WontonClient

private enum ShareExpiry: Int, CaseIterable {
    case indefinite, hour, today, custom

    var title: String {
        switch self {
        case .indefinite: String(localized: .Share0.shareIndefinite)
        case .hour: String(localized: .Share2.shareHour)
        case .today: String(localized: .Share1.shareToday)
        case .custom: String(localized: .Share0.shareCustom)
        }
    }
}

struct PluginShareEditor: View {
    let model: PluginsModel
    let pluginId: String
    let shares: PluginShares
    let grant: ShareGrant?
    let updated: (PluginDetail) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var recipients: FriendsViewModel?
    @State private var recipient: ShareRecipient?
    @State private var expiry: ShareExpiry = .indefinite
    @State private var date = Date.now.addingTimeInterval(3600)
    @State private var acknowledged = false
    @State private var problem: Problem?
    @State private var busy = false
    @State private var loading = true
    @State private var lastOperation: Operation = .load
    private enum Operation { case load, save, revoke }

    var body: some View {
        NavigationStack {
            Form {
                if loading { ProgressView() }
                if let recipients { recipientPicker(recipients) }
                Section(String(localized: .Share1.shareExpires)) {
                    Picker(String(localized: .Share1.shareExpires), selection: $expiry) {
                        ForEach(ShareExpiry.allCases, id: \.rawValue) { value in
                            Text(value.title).tag(value)
                        }
                    }
                    if expiry == .custom {
                        DatePicker(String(localized: .Share0.shareCustom), selection: $date)
                    }
                }
                if !shares.perUserAccounts {
                    Section {
                        WontonNotice(icon: .warning, title: String(localized: .Share2.shareRisk))
                        Toggle(String(localized: .Share1.shareAcknowledge), isOn: $acknowledged)
                    }
                }
                if let problem {
                    Section {
                        ProblemNotice(problem: problem)
                        Button(.Common.commonRetry) { perform(lastOperation) }
                    }
                }
                Section {
                    Button(.Share1.shareSave) { perform(.save) }.disabled(!canSave)
                    if grant != nil {
                        Button(.Share2.shareRevoke, role: .destructive) { perform(.revoke) }.disabled(busy)
                    }
                }
            }
            .disabled(busy)
            .navigationTitle(grant == nil ? String(localized: .Share2.shareAdd) : String(localized: .Share2.shareEdit))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(.Share0.shareCancel) { dismiss() }.disabled(busy)
                }
            }
            .task { initialize(); await load() }
        }
    }

    @ViewBuilder private func recipientPicker(_ view: FriendsViewModel) -> some View {
        Section(String(localized: .Share1.shareRecipient)) {
            if view.friends.isEmpty { Text(.Share1.shareEmpty) }
            Picker(String(localized: .Share1.shareRecipient), selection: $recipient) {
                Text(.Share1.shareRecipient).tag(ShareRecipient?.none)
                Section(String(localized: .Share2.shareFriends)) {
                    ForEach(view.friends, id: \.userId) { friend in
                        Text(friend.displayName.flatMap { $0.isEmpty ? nil : $0 } ?? friend.userId)
                            .tag(ShareRecipient?.some(.friend(userId: friend.userId)))
                    }
                }
            }
        }
    }

    private var canSave: Bool {
        !busy && !loading && recipientAvailable && (shares.perUserAccounts || acknowledged)
            && (expiry != .custom || date > .now)
    }

    private var recipientAvailable: Bool {
        guard let recipients, let recipient else { return false }
        switch recipient {
        case let .friend(id): return recipients.friends.contains { $0.userId == id }
        case .other: return false
        }
    }

    private func initialize() {
        recipient = grant?.recipient
        if let millis = grant?.expiresAtMs {
            expiry = .custom
            date = Date(timeIntervalSince1970: Double(millis) / 1000)
        }
        acknowledged = false
    }

    private func load() async {
        loading = true
        problem = nil
        do {
            recipients = try await model.core.shareRecipients()
            if !recipientAvailable { recipient = nil }
        } catch { problem = error.problem }
        loading = false
    }

    private func expiresAt() -> UInt64? {
        let value: Date
        switch expiry {
        case .indefinite: return nil
        case .hour: value = .now.addingTimeInterval(3600)
        case .today: value = Calendar.current.dateInterval(of: .day, for: .now)!.end
        case .custom: value = date
        }
        return UInt64(max(0, value.timeIntervalSince1970 * 1000))
    }

    private func perform(_ operation: Operation) {
        guard !busy else { return }
        busy = true
        lastOperation = operation
        problem = nil
        Task {
            defer { busy = false }
            if operation == .load { await load(); return }
            do {
                let detail: PluginDetail
                if operation == .revoke, let grant {
                    detail = try await model.core.revokePluginShare(id: grant.id, pluginId: pluginId)
                } else {
                    guard canSubmit, let recipient else { return }
                    detail = try await model.core.savePluginShare(id: grant?.id, grant: ShareGrantUpdate(
                        pluginId: pluginId, recipient: recipient, expiresAtMs: expiresAt(),
                        acknowledgeRisk: acknowledged
                    ))
                }
                updated(detail)
            } catch { problem = error.problem }
        }
    }

    private var canSubmit: Bool {
        !loading && recipientAvailable && (shares.perUserAccounts || acknowledged)
            && (expiry != .custom || date > .now)
    }
}
