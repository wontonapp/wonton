import SwiftUI
import WontonClient

/// 主屏图标下面的状态：连接、等确认的卡片、存储。不知道的如实显示不确定。
struct HomeStatusPanel: View {
    let home: HomeViewModel
    let plugins: PluginsViewModel
    let cards: [PendingConfirmation]
    let showConnection: () -> Void
    let openCardRoom: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        WontonGroup {
            Button(action: showConnection) {
                WontonRow(
                    title: "连接", value: connection, valueTint: needsTheOwner ? colors.danger : nil, showsForward: true
                )
            }
            .buttonStyle(.plain)
            WontonSeparator()
            if let first = cards.first {
                Button { openCardRoom(first.roomId) } label: {
                    WontonRow(title: "等确认", value: "\(cards.count) 张", showsForward: true)
                }
                .buttonStyle(.plain)
            } else {
                WontonRow(title: "等确认", value: "没有")
            }
            WontonSeparator()
            WontonRow(title: "存储", value: storage)
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
            return "\(home.status.connectionText) · \(path.relay ? "中继" : "直连")"
        default:
            return home.notice?.sentence ?? home.status.connectionText
        }
    }

    private var storage: String {
        guard let usage = plugins.storage else { return "不确定" }
        let free = ByteCountFormatter.string(fromByteCount: Int64(clamping: usage.freeBytes), countStyle: .file)
        let total = ByteCountFormatter.string(fromByteCount: Int64(clamping: usage.totalBytes), countStyle: .file)
        return "\(free) 可用，共 \(total)"
    }
}
