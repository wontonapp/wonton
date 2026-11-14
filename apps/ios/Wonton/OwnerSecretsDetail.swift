import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// Values live only in the core's expanded ViewModel; the shell never keeps a copy in state.
struct OwnerSecretsDetail: View {
    let model: PluginsModel
    let pluginId: String
    @State private var isOwner: Bool?
    @State private var identityProblem: Problem?
    @Environment(\.wontonColors) private var colors

    private var group: OwnerSecretsView? {
        guard model.viewModel.ownerSecrets?.pluginId == pluginId else { return nil }
        return model.viewModel.ownerSecrets
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            if isOwner == true {
                groupHeader
                if let group { contents(group) }
            } else if let identityProblem {
                DetailSection(title: String(localized: .ownerSecretsTitle)) {
                    ProblemNotice(problem: identityProblem)
                    Button(String(localized: .Common.commonRetry)) { Task { await loadIdentity() } }
                        .buttonStyle(WontonSmallButtonStyle())
                }
            } else if isOwner == nil {
                ProgressView()
            }
        }
        .task(id: pluginId) { await loadIdentity() }
        .onDisappear { model.send(.dismissOwnerSecrets(pluginId: pluginId)) }
    }

    private var groupHeader: some View {
        Button {
            if group == nil {
                model.send(.listOwnerSecrets(pluginId: pluginId))
            } else {
                model.send(.dismissOwnerSecrets(pluginId: pluginId))
            }
        } label: {
            HStack {
                Text(.ownerSecretsTitle)
                    .wontonText(WontonTheme.typography.sectionTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                Spacer(minLength: 0)
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
                    .rotationEffect(group == nil ? .zero : .degrees(90))
            }
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(.isHeader)
        .accessibilityValue(
            String(localized: group == nil ? .Appearance0.appearanceCollapsed : .Appearance1.appearanceExpanded)
        )
    }

    @ViewBuilder private func contents(_ group: OwnerSecretsView) -> some View {
        if group.loading {
            ProgressView()
        } else if let problem = group.problem {
            ProblemNotice(problem: problem.ownerSecretProblem)
            Button(String(localized: .Common.commonRetry)) { model.send(.listOwnerSecrets(pluginId: pluginId)) }
                .buttonStyle(WontonSmallButtonStyle())
        } else if group.secrets.isEmpty {
            Text(.ownerSecretsEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        } else {
            ForEach(group.secrets, id: \.name) { secret in
                OwnerSecretRow(model: model, pluginId: pluginId, secret: secret)
            }
        }
    }

    private func loadIdentity() async {
        isOwner = nil
        identityProblem = nil
        do {
            let profile = try await model.core.me()
            guard !Task.isCancelled else { return }
            isOwner = profile.isOwner
        } catch {
            guard !Task.isCancelled else { return }
            identityProblem = error.problem
        }
    }
}

private struct OwnerSecretRow: View {
    let model: PluginsModel
    let pluginId: String
    let secret: OwnerSecretView
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(verbatim: secret.label)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(String(localized: secret.expanded ? .ownerSecretsHide : .ownerSecretsShow)) {
                    if secret.expanded {
                        model.send(.hideOwnerSecret(pluginId: pluginId, name: secret.name))
                    } else {
                        show()
                    }
                }
                .buttonStyle(WontonSmallButtonStyle())
            }
            if secret.expanded {
                if secret.loading {
                    ProgressView()
                } else if let problem = secret.problem {
                    ProblemNotice(problem: problem.ownerSecretProblem)
                    Button(String(localized: .Common.commonRetry), action: show)
                        .buttonStyle(WontonSmallButtonStyle())
                } else if let value = secret.value {
                    Text(verbatim: value)
                        .font(.body.monospaced())
                        .foregroundStyle(Color(colors.textPrimary))
                        .padding(WontonTheme.spacing.s12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            RoundedRectangle(cornerRadius: WontonTheme.radii.control)
                                .fill(Color(colors.surfaceSubtle))
                        )
                    // Like the recovery key, copying must not bypass the local-only clipboard protection.
                    Button(String(localized: .ChatBackup1.chatBackupCopy)) {
                        UIPasteboard.general.setItems(
                            [[UTType.plainText.identifier: value]],
                            options: [.localOnly: true, .expirationDate: Date.now.addingTimeInterval(120)]
                        )
                    }
                    .buttonStyle(WontonSmallButtonStyle())
                }
            }
        }
    }

    private func show() {
        model.send(.showOwnerSecret(pluginId: pluginId, name: secret.name))
    }
}

private extension OwnerSecretProblem {
    var ownerSecretProblem: Problem {
        let title: LocalizedStringResource = switch reason {
        case .transportUnavailable: .Problem2.problemTransportUnavailableTitle
        case .unauthenticated: .Problem3.problemUnauthorizedTitle
        case .crossSite: .ownerSecretsProblemCrossSite
        case .permissionRequired: .ownerSecretsProblemOwnerOnly
        case .notInstalled: .ownerSecretsProblemNotInstalled
        case .notFound: .ownerSecretsProblemNotFound
        case .notReady: .ownerSecretsProblemNotReady
        case .upstream: .Problem2.problemUpstreamTitle
        case .protocol: .ownerSecretsProblemProtocol
        default: .ownerSecretsProblemOther
        }
        return Problem(title: String(localized: title), detail: message)
    }
}
