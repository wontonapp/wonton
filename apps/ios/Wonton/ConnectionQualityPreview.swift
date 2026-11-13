#if DEBUG
import SwiftUI
import WontonClient

/// 固定 ViewModel 的界面走查，不替代真实隧道验证。
/// 模拟器截图：`xcrun simctl launch <sim> app.wonton.ios wonton-quality-preview[-relay|-details]`，
/// 再 `xcrun simctl io <sim> screenshot <文件>`。
enum QualityPreviews {
    /// 启动参数带 `wonton-quality-preview` 时，App 不进主界面，直接渲染走查屏。
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains { $0.hasPrefix("wonton-quality-preview") }
    }

    @ViewBuilder
    static var root: some View {
        switch ProcessInfo.processInfo.arguments.first(where: { $0.hasPrefix("wonton-quality-preview") }) {
        case "wonton-quality-preview-relay": QualityHintPreview(home: fakeHome(relay: true), chat: true)
        case "wonton-quality-preview-details": QualityDetailsPreview()
        case "wonton-quality-preview-udp": UdpDetailsPreview(evicted: 3)
        case "wonton-quality-preview-udp-unknown": UdpDetailsPreview(evicted: nil)
        case "wonton-quality-preview-udp-zero": UdpDetailsPreview(evicted: 0)
        case "wonton-quality-preview-udp-large": UdpDetailsPreview(evicted: .max)
        default: QualityHintPreview(home: fakeHome(relay: false), chat: false)
        }
    }

    /// Poor 且连着的假 ViewModel，数字取 11-08 中继劣化的实测量级（丢包约 7%，往返约 1.2 秒）。
    static func fakeHome(relay: Bool) -> HomeViewModel {
        HomeViewModel(
            deviceId: "0", serverId: nil, status: .connected,
            path: HomePath(relay: relay, publicRelay: false, lan: false, rttMs: 1_240), notice: nil,
            details: ConnectionDetails(
                network: .wifi, relay: .ok, server: .ok, lastConnectedAtMs: 1_794_000_000_000,
                handshakeMs: 120, disconnects: 0, failedDials: 0, sentPackets: 900, lostPackets: 66,
                quality: .poor, lossPerMille: 73, rttMs: 1_240,
                lastFailure: nil, addressesStale: false, udpFlows: [], udpBufferEvictedDatagrams: nil
            ),
            proxy: nil, webUrl: nil, pairing: nil, invite: nil, keepConnection: false, justRecovered: false
        )
    }
}

/// 轻提示的位置：根页面替换页面标题，聊天房间是副标题。
private struct QualityHintPreview: View {
    let home: HomeViewModel
    let chat: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            if chat {
                Color(colors.canvas).ignoresSafeArea()
                    .wontonPushedScreen(String(localized: .Plugins.previewBotName), showsStatus: true) { EmptyView() }
            } else {
                Color(colors.canvas).ignoresSafeArea()
                    .wontonRootScreen("Wonton", hasActions: false) { EmptyView() }
            }
        }
        .environment(\.wontonBarStatus, WontonBarStatus(home: home) { _ in
            // 走查只覆盖 Poor：带下一步的长通知不在这三种状态里。
            EmptyView()
        })
    }
}

/// 详情页：三行质量指标有值时的样子。
private struct QualityDetailsPreview: View {
    private let home = QualityPreviews.fakeHome(relay: true)

    var body: some View {
        ScrollView {
            ConnectionDetailsRows(details: home.details, path: home.path)
                .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .Common.commonDetails))
    }
}

private struct UdpDetailsPreview: View {
    let evicted: UInt64?

    private var home: HomeViewModel {
        var home = QualityPreviews.fakeHome(relay: false)
        home.details.lastConnectedAtMs = nil
        home.details.udpFlows = [
            UdpFlowStats(
                flowId: 1, pluginId: "LiveKit", port: 7882,
                receivedPackets: 240, sentPackets: 120, oversizedDrops: 0, bufferEvictedDrops: nil
            ),
            UdpFlowStats(
                flowId: 2, pluginId: "Neko", port: 52000,
                receivedPackets: evicted == .max ? .max : 900,
                sentPackets: 800, oversizedDrops: 2, bufferEvictedDrops: nil
            )
        ]
        home.details.udpBufferEvictedDatagrams = evicted
        return home
    }

    var body: some View {
        ScrollView {
            ConnectionDetailsRows(details: home.details, path: home.path)
                .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .Common.commonDetails))
    }
}

#Preview("Poor") {
    QualityHintPreview(home: QualityPreviews.fakeHome(relay: false), chat: false)
}

#Preview("Poor relay") {
    QualityHintPreview(home: QualityPreviews.fakeHome(relay: true), chat: true)
}

#Preview("Details") {
    QualityDetailsPreview()
}
#endif
