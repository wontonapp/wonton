import SwiftUI
import WontonClient

struct SharedAppsSection: View {
    let model: PluginsModel

    var body: some View {
        DetailSection(title: String(localized: .Share1.shareReceived)) {
            Button(.Share1.shareRefresh) { model.send(.refreshSharedApps(ownerUserId: nil)) }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            if model.viewModel.sharedApps == nil { ProgressView() }
            ForEach(model.viewModel.sharedApps?.sources ?? [], id: \.ownerUserId) { source in
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    Text(source.ownerDisplayName.flatMap { $0.isEmpty ? nil : $0 } ?? source.ownerUserId)
                        .wontonText(WontonTheme.typography.itemTitle)
                    if let name = source.ownerDisplayName, !name.isEmpty {
                        Text(source.ownerUserId).wontonText(WontonTheme.typography.caption)
                    }
                    Button(.Share1.shareRefresh) { model.send(.refreshSharedApps(ownerUserId: source.ownerUserId)) }
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
            ProblemRow(problem: Problem(title: String(localized: .Share0.shareProblemOther))) {
                model.send(.refreshSharedApps(ownerUserId: source.ownerUserId))
            }
        }
    }
}

extension ShareProblem {
    var problem: Problem {
        let title: LocalizedStringResource
        switch reason {
        case .unreachable: title = .Share2.shareProblemUnreachable
        case .identity: title = .Share0.shareProblemIdentity
        case .required: title = .Share0.shareProblemRequired
        case .revoked: title = .Share0.shareProblemRevoked
        case .expired: title = .Share0.shareProblemExpired
        case .pluginNotRunning: title = .Share1.shareProblemPlugin
        case .other: title = .Share0.shareProblemOther
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
        case .invalidToken: String(localized: .Share1.shareIdentityInvalid)
        case .discoveryFailed: String(localized: .Share1.shareIdentityDiscovery)
        case .userInfoFailed: String(localized: .Share0.shareIdentityUserinfo)
        case .serverNameMismatch: String(localized: .Share0.shareIdentityMismatch)
        case .other: String(localized: .Share1.shareIdentityOther)
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
            return Problem(title: String(localized: .Share2.shareProblemUnreachable))
        }
        if case let .failed(problem) = source.state { return problem.problem }
        if case .other = source.state { return Problem(title: String(localized: .Share0.shareProblemOther)) }
        return source.apps.first { $0.plugin.pluginId == pluginId }?.problem?.problem
    }
}
