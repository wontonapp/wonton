import SwiftUI
import WontonClient

struct SharedAppsSection: View {
    let model: PluginsModel

    var body: some View {
        DetailSection(title: String(localized: .shareReceived)) {
            Button(.shareRefresh) { model.send(.refreshSharedApps(ownerUserId: nil)) }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            if model.viewModel.sharedApps == nil { ProgressView() }
            ForEach(model.viewModel.sharedApps?.sources ?? [], id: \.ownerUserId) { source in
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    Text(source.ownerDisplayName.flatMap { $0.isEmpty ? nil : $0 } ?? source.ownerUserId)
                        .wontonText(WontonTheme.typography.itemTitle)
                    if let name = source.ownerDisplayName, !name.isEmpty {
                        Text(source.ownerUserId).wontonText(WontonTheme.typography.caption)
                    }
                    Button(.shareRefresh) { model.send(.refreshSharedApps(ownerUserId: source.ownerUserId)) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                    sourceState(source)
                    ForEach(source.apps, id: \.plugin.pluginId) { row in
                        Button(row.plugin.title) {
                            model.send(.openSharedApp(ownerUserId: source.ownerUserId, pluginId: row.plugin.pluginId))
                        }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                        if let problem = row.problem { ProblemNotice(problem: problem.problem) }
                    }
                }
            }
        }
    }

    @ViewBuilder private func sourceState(_ source: SharedAppsSource) -> some View {
        switch source.state {
        case .loading: ProgressView()
        case .ready: EmptyView()
        case let .failed(problem):
            ProblemRow(problem: problem.problem) { model.send(.refreshSharedApps(ownerUserId: source.ownerUserId)) }
        case .other:
            ProblemRow(problem: Problem(title: String(localized: .shareProblemOther))) {
                model.send(.refreshSharedApps(ownerUserId: source.ownerUserId))
            }
        }
    }
}

extension ShareProblem {
    var problem: Problem {
        let title: LocalizedStringResource
        switch reason {
        case .unreachable: title = .shareProblemUnreachable
        case .identity: title = .shareProblemIdentity
        case .required: title = .shareProblemRequired
        case .revoked: title = .shareProblemRevoked
        case .expired: title = .shareProblemExpired
        case .pluginNotRunning: title = .shareProblemPlugin
        case .other: title = .shareProblemOther
        }
        var details: [String] = []
        if let identity { details.append(identity.shareDetail) }
        if let httpStatus { details.append("HTTP \(httpStatus)") }
        return Problem(title: String(localized: title), detail: details.isEmpty ? nil : details.joined(separator: "\n"))
    }
}

private extension ShareIdentityFailure {
    var shareDetail: String {
        switch self {
        case .invalidToken: String(localized: .shareIdentityInvalid)
        case .discoveryFailed: String(localized: .shareIdentityDiscovery)
        case .userInfoFailed: String(localized: .shareIdentityUserinfo)
        case .serverNameMismatch: String(localized: .shareIdentityMismatch)
        case .other: String(localized: .shareIdentityOther)
        }
    }
}

extension PluginsViewModel {
    func sharedSource(for tab: TabRef) -> SharedAppsSource? {
        guard case let .sharedApp(ownerUserId, _) = tab else { return nil }
        return sharedApps?.sources.first { $0.ownerUserId == ownerUserId }
    }

    func sharedProblem(for tab: TabRef) -> Problem? {
        guard case let .sharedApp(_, pluginId) = tab else { return nil }
        guard let source = sharedSource(for: tab) else {
            return Problem(title: String(localized: .shareProblemUnreachable))
        }
        if case let .failed(problem) = source.state { return problem.problem }
        if case .other = source.state { return Problem(title: String(localized: .shareProblemOther)) }
        return source.apps.first { $0.plugin.pluginId == pluginId }?.problem?.problem
    }
}
