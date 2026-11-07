import SwiftUI
import WontonClient

/// 空列表的一句话加两个按钮。
struct PeersEmpty: View {
    let showCode: () -> Void
    let scan: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text(.peersEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.peersShowCodeButton, action: showCode)
                .buttonStyle(WontonButtonStyle())
            Button(.peersScanCodeButton, action: scan)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
        }
        .frame(maxWidth: .infinity)
        .padding(.top, WontonTheme.spacing.s48)
    }
}

/// 一台配对过的服务器。正常（`Connected`）不写状态；删除在左滑，要主人再确认的行内也给“移除”。
struct PeerRowView: View {
    let row: PeerRow
    let remove: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(name)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .layoutPriority(1)
                Spacer(minLength: 0)
                if showsRemove {
                    Button(String(localized: .commonRemove), action: remove)
                        .buttonStyle(WontonSmallButtonStyle(kind: .danger))
                }
            }
            statusLine
            if row.status == .offline, let last = row.lastConnectedMs {
                Text(.peersLastConnected(relative(last)))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            ProblemDetail(text: details)
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }

    /// 对方断开后要主人清掉这条；等对方同意的行“移除”是取消请求；对方拒绝的行移除这条记录。
    private var showsRemove: Bool {
        switch row.status {
        case .removedByPeer, .awaitingApproval, .declined: true
        case .checking, .connected, .offline, .other: false
        }
    }

    private var name: String {
        row.ownerDisplayName.isEmpty ? row.serverName : row.ownerDisplayName
    }

    @ViewBuilder private var statusLine: some View {
        let status: LocalizedStringResource? = switch row.status {
        case .checking: .peersStatusChecking
        case .connected: nil
        case .offline: .peersStatusOffline
        case .removedByPeer: .peersStatusRemoved
        case .awaitingApproval: .peersStatusAwaiting
        case .declined: .peersStatusDeclined
        case .other: .peersStatusUnknown
        }
        if let status {
            Text(status)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
    }

    /// 默认收起的“详情”：服务器名和核心写的最后一次错误。
    private var details: String {
        peerDetails(serverName: row.serverName, detail: row.detail)
    }

    private func relative(_ millis: UInt64) -> String {
        relativeTime(millis)
    }
}

/// 一条别人发来的配对请求：对方的显示名、重名提示、收到的时间和同意、拒绝，服务器在“详情”里。
struct PeerRequestRowView: View {
    let request: PeerRequestRow
    let approve: () -> Void
    let reject: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(name)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .layoutPriority(1)
                Spacer(minLength: 0)
            }
            if let conflict = request.conflictsWith {
                Label {
                    Text(.peersConflict(conflict))
                } icon: {
                    WontonIcon.warning.view(size: WontonTheme.size.iconSmall)
                }
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.warning))
            }
            Text(.peersReceived(relativeTime(request.receivedAtMs)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(.peersApprove, action: approve)
                    .buttonStyle(WontonSmallButtonStyle())
                Button(.peersReject, action: reject)
                    .buttonStyle(WontonSmallButtonStyle(kind: .secondary))
            }
            .padding(.top, WontonTheme.spacing.s4)
            ProblemDetail(text: peerDetails(serverName: request.serverName, detail: request.detail))
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }

    private var name: String {
        request.ownerDisplayName.isEmpty ? request.serverName : request.ownerDisplayName
    }
}

/// 一条主人拒绝过的请求，能撤销。
struct RejectedRequestRowView: View {
    let row: RejectedPeerRow
    let unreject: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(name)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .layoutPriority(1)
                Spacer(minLength: 0)
                Button(.peersUndo, action: unreject)
                    .buttonStyle(WontonSmallButtonStyle())
            }
            Text(.peersRejectedAt(relativeTime(row.rejectedAtMs)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            ProblemDetail(text: peerDetails(serverName: row.serverName, detail: row.detail))
        }
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target, alignment: .leading)
    }

    private var name: String {
        row.ownerDisplayName.isEmpty ? row.serverName : row.ownerDisplayName
    }
}

/// 几处“详情”共用的内容：服务器名和核心写的说明。
private func peerDetails(serverName: String, detail: String?) -> String {
    var lines = [serverName]
    if let detail, !detail.isEmpty { lines.append(detail) }
    return lines.joined(separator: "\n")
}

private func relativeTime(_ millis: UInt64) -> String {
    Date(timeIntervalSince1970: TimeInterval(millis) / 1000).formatted(.relative(presentation: .named))
}
