import SwiftUI
import WontonClient

/// 主屏图标下面的状态：连接、等确认的卡片、存储。不知道的如实显示不确定。
struct HomeStatusPanel: View {
    let home: HomeViewModel
    let plugins: PluginsViewModel
    let cards: [PendingConfirmation]
    let showConnection: () -> Void
    let openCardRoom: (String?) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        WontonGroup {
            Button(action: showConnection) {
                WontonRow(
                    title: String(localized: .HomeStatus.homeStatusConnection), value: connection,
                    valueTint: needsTheOwner ? colors.danger : nil, showsForward: true
                )
            }
            .buttonStyle(.plain)
            WontonSeparator()
            if let first = cards.first {
                Button { openCardRoom(first.roomId) } label: {
                    WontonRow(
                        title: String(localized: .HomeStatus.homeStatusPending),
                        value: String(localized: .HomeStatus.homeStatusPendingCount(count: cards.count)),
                        showsForward: true
                    )
                }
                .buttonStyle(.plain)
            } else {
                WontonRow(title: String(localized: .HomeStatus.homeStatusPending),
                    value: String(localized: .HomeStatus.homeStatusNone))
            }
            WontonSeparator()
            WontonRow(title: String(localized: .HomeStatus.homeStatusStorage), value: storage)
            if let problem = plugins.storageProblem {
                Text(problem.message)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, WontonTheme.spacing.s16)
                    .padding(.bottom, WontonTheme.spacing.s12)
            }
        }
    }

    private var needsTheOwner: Bool {
        home.notice == .deviceRejected || home.notice == .versionMismatch
    }

    private var connection: String {
        switch home.status {
        case .connected:
            guard let path = home.path else { return home.status.connectionText }
            let route = path.relay
                ? String(localized: .Connection0.connectionRelay)
                : String(localized: .Connection2.connectionDirect)
            return "\(home.status.connectionText) · \(route)"
        default:
            return home.notice?.sentence ?? home.status.connectionText
        }
    }

    private var storage: String {
        guard let usage = plugins.storage else { return String(localized: .Common.commonUnknown) }
        let free = ByteCountFormatter.string(fromByteCount: Int64(clamping: usage.freeBytes), countStyle: .file)
        let total = ByteCountFormatter.string(fromByteCount: Int64(clamping: usage.totalBytes), countStyle: .file)
        return String(localized: .HomeStatus.homeStatusStorageUsage(free, total))
    }
}
