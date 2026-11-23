#if DEBUG
import SwiftUI
import WontonClient

/// 界面验收的“其他服务器”页夹具：启动参数带 `wonton-peers-preview` 时生效（同 `ProblemPreview` 的做法）。
/// 假数据只为看几行状态怎么画：连上的不写状态，公网关着和掉线各占一行。
enum PeersPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-peers-preview")
    }

    static var root: some View {
        peersScreen
    }

    private static var peersScreen: some View {
        List {
            Section {
                ForEach(rows, id: \.serverName) { row in
                    PeerRowView(row: row, remove: {})
                }
            } header: {
                Text(.Peers3.peersTitle)
            }
        }
        .listStyle(.insetGrouped)
    }

    private static var rows: [PeerRow] {
        [
            PeerRow(
                serverName: "lin7c3d.wonton",
                ownerUserId: "@lin:lin7c3d.wonton",
                ownerDisplayName: "Lin",
                status: .connected,
                lastConnectedMs: 1_762_800_000_000,
                detail: nil
            ),
            PeerRow(
                serverName: "sam5e8f.wonton",
                ownerUserId: "@sam:sam5e8f.wonton",
                ownerDisplayName: "Sam",
                status: .publicAccessOff,
                lastConnectedMs: 1_762_700_000_000,
                detail: endpoint("3c4d5e6f")
            ),
            PeerRow(
                serverName: "mai2a9b.wonton",
                ownerUserId: "@mai:mai2a9b.wonton",
                ownerDisplayName: "Mai",
                status: .offline,
                lastConnectedMs: 1_762_600_000_000,
                detail: endpoint("7a8b9c0d")
            )
        ]
    }

    private static func endpoint(_ id: String) -> String { "endpoint \(id)" }
}
#endif
