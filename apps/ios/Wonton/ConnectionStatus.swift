import Network
import SwiftUI
import WontonClient

extension HomeStatus {
    var connectionText: String {
        switch self {
        case .notConfigured: String(localized: .connectionNotPaired)
        case .connected: String(localized: .connectionConnected)
        case .connecting: ConnectionNotice.connecting.sentence
        case .disconnected: ConnectionNotice.unreachable(retrying: false).sentence
        }
    }
}

extension ConnectionNotice {
    var sentence: String {
        switch self {
        case .waitingForNetwork: String(localized: .connectionNoticeWaitingForNetwork)
        case .connecting: String(localized: .connectionNoticeConnecting)
        case .updating: String(localized: .connectionNoticeUpdating)
        case .serverOffline: String(localized: .connectionNoticeServerOffline)
        case .unreachable: String(localized: .connectionNoticeUnreachable)
        case .deviceRejected: String(localized: .connectionNoticeDeviceRejected)
        case .versionMismatch: String(localized: .connectionNoticeVersionMismatch)
        }
    }

    fileprivate var isShort: Bool {
        switch self {
        case .waitingForNetwork, .connecting, .updating: true
        case .serverOffline, .unreachable, .deviceRejected, .versionMismatch: false
        }
    }

    /// 能重试时给出是否正在重试。
    var retrying: Bool? {
        switch self {
        case let .serverOffline(retrying), let .unreachable(retrying): retrying
        case .waitingForNetwork, .connecting, .updating, .deviceRejected, .versionMismatch: nil
        }
    }
}

extension WontonBarStatus {
    init<Line: View>(notice: ConnectionNotice?, @ViewBuilder line: (ConnectionNotice) -> Line) {
        self.init(
            title: notice.flatMap { $0.isShort ? $0.sentence : nil },
            subtitle: notice?.sentence,
            line: notice.flatMap { $0.isShort ? nil : AnyView(line($0)) }
        )
    }
}

/// 长时间连不上：一句话和下一步。不用红色：还在自动重试，也不是主人做错了什么；要主人动手的用警告色。
struct ConnectionLine: View {
    let notice: ConnectionNotice
    let retry: () -> Void
    let open: (SettingsRoute) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(notice.sentence)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(notice == .deviceRejected ? colors.warning : colors.textPrimary))
            HStack(spacing: WontonTheme.spacing.s8) {
                if notice == .deviceRejected {
                    Button(.connectionPairAgain) { open(.pairing) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                } else {
                    if let retrying = notice.retrying { RetryButton(retrying: retrying, retry: retry) }
                    Button(.connectionViewConnection) { open(.server) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct RetryButton: View {
    let retrying: Bool
    let retry: () -> Void

    var body: some View {
        Button(action: retry) {
            HStack(spacing: WontonTheme.spacing.s4) {
                if retrying { ProgressView().controlSize(.small) }
                Text(.commonRetry)
            }
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
        .disabled(retrying)
    }
}

extension DeviceNetwork {
    /// 系统报告的网络变化，第一项是当前的网络。壳不做判断，都交给核心（ADR-0032）。
    static func changes() -> AsyncStream<DeviceNetwork> {
        AsyncStream { continuation in
            let monitor = NWPathMonitor()
            monitor.pathUpdateHandler = { path in
                continuation.yield(DeviceNetwork(path))
            }
            monitor.start(queue: DispatchQueue(label: "app.wonton.network"))
            continuation.onTermination = { _ in monitor.cancel() }
        }
    }

    private init(_ path: NWPath) {
        if path.status != .satisfied {
            self = .none
        } else if path.usesInterfaceType(.wifi) {
            self = .wifi
        } else if path.usesInterfaceType(.cellular) {
            self = .cellular
        } else {
            self = .other
        }
    }
}

/// 和家里服务器的连接、诊断和管理入口。
struct ConnectionView: View {
    let model: HomeModel
    let plugins: PluginsModel
    @Environment(\.wontonColors) private var colors
    @State private var removing: ServerRow?

    private var view: HomeViewModel { model.viewModel }

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                ServersSection(model: model, removing: $removing)
                WontonGroup {
                    WontonRow(title: String(localized: .connectionCellular), value: view.details.network.check.text)
                    WontonSeparator()
                    WontonRow(title: String(localized: .connectionRelay), value: view.details.relay.text)
                    WontonSeparator()
                    WontonRow(title: String(localized: .egressPathHomeServer), value: view.details.server.text)
                }
                if let retrying = view.notice?.retrying {
                    RetryButton(retrying: retrying) { model.send(.retryConnection) }
                }
                WontonGroup {
                    NavigationLink(value: SettingsRoute.connectionDetails) {
                        WontonRow(title: String(localized: .commonDetails), showsForward: true)
                    }
                }
                .buttonStyle(.plain)
                WontonGroup {
                    NavigationLink(value: SettingsRoute.egress) {
                        WontonRow(title: String(localized: .egressTitle), showsForward: true)
                    }
                    WontonSeparator()
                    NavigationLink(value: SettingsRoute.registryMirror) {
                        WontonRow(title: String(localized: .registryMirrorTitle), showsForward: true)
                    }
                    WontonSeparator()
                    NavigationLink(value: SettingsRoute.coreUpgrade) {
                        WontonRow(
                            title: String(localized: .coreUpgradeTitle),
                            value: plugins.viewModel.coreUpgrade.isPrompting
                                ? String(localized: .designUpdateBadge) : nil,
                            showsForward: true
                        )
                    }
                }
                .buttonStyle(.plain)
                WontonGroup {
                    NavigationLink(value: SettingsRoute.invite) {
                        WontonRow(title: String(localized: .inviteTitle), showsForward: true)
                    }
                }
                .buttonStyle(.plain)
                WontonGroup {
                    NavigationLink(value: SettingsRoute.pairing) {
                        WontonRow(
                            title: String(localized: .connectionPairAgain),
                            titleTint: colors.danger, showsForward: true
                        )
                    }
                }
                .buttonStyle(.plain)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .removeServerDialog($removing, model: model)
        .wontonPushedScreen(String(localized: .egressPathHomeServer))
    }
}

struct ConnectionDetailsView: View {
    let model: HomeModel

    private var details: ConnectionDetails { model.viewModel.details }

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    if details.addressesStale {
                        WontonRow(title: String(localized: .connectionAddressesStale))
                        WontonSeparator()
                    }
                    if let path = model.viewModel.path {
                        WontonRow(title: String(localized: .connectionDetailPath), value: path.kindText)
                        WontonSeparator()
                        WontonRow(title: String(localized: .connectionDetailLatency), value: "\(path.rttMs) ms")
                        WontonSeparator()
                    }
                    WontonRow(
                        title: String(localized: .connectionDetailLoss),
                        value: "\(details.lostPackets) / \(details.sentPackets)"
                    )
                    if let handshake = details.handshakeMs {
                        WontonSeparator()
                        WontonRow(title: String(localized: .connectionDetailLastHandshake), value: "\(handshake) ms")
                    }
                    WontonSeparator()
                    WontonRow(title: String(localized: .connectionDetailDisconnects), value: "\(details.disconnects)")
                    if let connected = details.lastConnectedAtMs {
                        WontonSeparator()
                        WontonRow(title: String(localized: .connectionDetailLastConnected), value: Self.time(connected))
                    }
                }
                if let failure = details.lastFailure {
                    WontonGroup {
                        WontonRow(title: String(localized: .connectionDetailLastFailure), value: failure)
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .commonDetails))
    }

    private static func time(_ milliseconds: UInt64) -> String {
        Date(timeIntervalSince1970: Double(milliseconds) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}

private extension HomePath {
    var kindText: String {
        if lan { return String(localized: .connectionLan) }
        return relay ? String(localized: .connectionRelay) : String(localized: .connectionDirect)
    }
}

private extension Optional<DeviceNetwork> {
    /// 连接是否走得通以核心为准，这一行只说系统怎么讲。
    var check: ConnectionCheck {
        switch self {
        case nil: .unknown
        case .some(.none): .failing
        case .some: .ok
        }
    }
}

private extension ConnectionCheck {
    var text: String {
        switch self {
        case .ok: String(localized: .connectionCheckOk)
        case .failing: String(localized: .connectionCheckFailing)
        case .unknown: String(localized: .commonUnknown)
        }
    }
}
