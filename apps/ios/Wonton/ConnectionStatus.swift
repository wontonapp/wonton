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
        case .waitingForNetwork, .updating: true
        case .connecting, .serverOffline, .unreachable, .deviceRejected, .versionMismatch: false
        }
    }

    /// 以核心通知为准，避免 HomeStatus.Connecting 掩盖超时后恢复的重试入口。
    var retrying: Bool? {
        switch self {
        case let .serverOffline(retrying), let .unreachable(retrying): retrying
        case .connecting: true
        case .waitingForNetwork, .updating, .deviceRejected, .versionMismatch: nil
        }
    }
}

/// 质量差的轻提示（#847）：直连和经中继各一条完整文案，不拼句子。
enum ConnectionQualityHint: Equatable {
    case poor
    case poorRelay

    var sentence: String {
        switch self {
        case .poor: String(localized: .connectionNoticePoor)
        case .poorRelay: String(localized: .connectionNoticePoorRelay)
        }
    }

    /// 只在连接着、核心判了 Poor、又没有别的通知时出现，占 ADR-0032 短状态的位置；Unknown、Good、Other 都不显示。
    /// 判断是纯函数：哪种状态显示哪一条，好测。
    static func current(
        connected: Bool, notice: ConnectionNotice?, quality: ConnectionQuality, pathRelay: Bool?
    ) -> ConnectionQualityHint? {
        guard connected, notice == nil, quality == .poor else { return nil }
        return pathRelay == true ? .poorRelay : .poor
    }
}

extension WontonBarStatus {
    /// 同一时刻只显示一条：核心的通知优先，没有通知且质量差时轮到轻提示。
    init<Line: View>(home: HomeViewModel, recovered: Bool = false, @ViewBuilder line: (ConnectionNotice) -> Line) {
        let hint = ConnectionQualityHint.current(
            connected: home.status == .connected,
            notice: home.notice,
            quality: home.details.quality,
            pathRelay: home.path?.relay
        )
        let connected = recovered && home.notice == nil ? String(localized: .connectionConnected) : nil
        self.init(
            title: home.notice.flatMap { $0.isShort ? $0.sentence : nil }
                ?? connected ?? hint?.sentence,
            subtitle: home.notice?.sentence ?? connected ?? hint?.sentence,
            line: home.notice.flatMap { $0.isShort ? nil : AnyView(line($0)) }
        )
    }
}

/// 进度与失败原因互斥，等待结束后由核心重新给出原因和下一步。
struct ConnectionLine: View {
    let notice: ConnectionNotice
    let retry: () -> Void
    let open: (SettingsRoute) -> Void
    @Environment(\.wontonColors) private var colors

    private var retrying: Bool { notice.retrying == true }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            if retrying {
                HStack(spacing: WontonTheme.spacing.s8) {
                    ProgressView().controlSize(.small)
                    Text(.connectionNoticeRetrying)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                }
            } else {
                Text(notice.sentence)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(notice == .deviceRejected ? colors.warning : colors.textPrimary))
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                if notice == .deviceRejected {
                    Button(.connectionPairAgain) { open(.pairing) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                } else {
                    if notice.retrying == false {
                        Button(.commonRetry, action: retry)
                            .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
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
                ConnectionDetailsRows(details: details, path: model.viewModel.path)
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
}

/// 详情页的行。窗口化的质量指标（#847）样本不够时显示"—"，不是 0。
struct ConnectionDetailsRows: View {
    let details: ConnectionDetails
    let path: HomePath?

    private static let missing = "—"

    var body: some View {
        WontonGroup {
            if details.addressesStale {
                WontonRow(title: String(localized: .connectionAddressesStale))
                WontonSeparator()
            }
            if let path {
                WontonRow(title: String(localized: .connectionDetailPath), value: path.kindText)
                WontonSeparator()
            }
            WontonRow(title: String(localized: .connectionDetailQuality), value: details.quality.text)
            WontonSeparator()
            WontonRow(
                title: String(localized: .connectionDetailLossRate),
                value: details.lossPerMille.map(Self.lossPercent) ?? Self.missing
            )
            WontonSeparator()
            WontonRow(
                title: String(localized: .connectionDetailLatency),
                value: details.rttMs.map { "\($0) ms" } ?? Self.missing
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
    }

    /// 千分比换算成百分比，保留一位小数（`73` 是 7.3%）。
    private static func lossPercent(_ perMille: UInt16) -> String {
        String(format: "%.1f%%", Double(perMille) / 10)
    }

    private static func time(_ milliseconds: UInt64) -> String {
        Date(timeIntervalSince1970: Double(milliseconds) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}

private extension ConnectionQuality {
    /// Other 是比这个 App 新的取值，按 Unknown 显示。
    var text: String {
        switch self {
        case .good: String(localized: .connectionQualityGood)
        case .poor: String(localized: .connectionQualityPoor)
        case .unknown, .other: String(localized: .commonUnknown)
        }
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
