import Network
import SwiftUI
import WontonClient

extension ConnectionNotice {
    var sentence: String {
        switch self {
        case .waitingForNetwork: "等待网络…"
        case .connecting: "连接中…"
        case .updating: "正在更新…"
        case .serverOffline: "家里的服务器好像离线了"
        case .unreachable: "连不上家里的服务器"
        case .deviceRejected: "这台手机已不被家里的服务器认可"
        case .versionMismatch: "App 或家里的服务器需要更新"
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
    let open: (HomeRoute) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(notice.sentence)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(notice == .deviceRejected ? colors.warning : colors.textPrimary))
            HStack(spacing: WontonTheme.spacing.s8) {
                if notice == .deviceRejected {
                    Button("重新配对") { open(.pairing) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                } else {
                    if let retrying = notice.retrying { RetryButton(retrying: retrying, retry: retry) }
                    Button("查看连接") { open(.connection) }
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
                Text("重试")
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

/// 和家里服务器的连接：三行诊断，以及添加设备、重新配对。
struct ConnectionView: View {
    let model: HomeModel
    let plugins: PluginsModel

    private var view: HomeViewModel { model.viewModel }

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    WontonRow(title: "手机网络", value: view.details.network.check.text)
                    WontonSeparator()
                    WontonRow(title: "中继", value: view.details.relay.text)
                    WontonSeparator()
                    WontonRow(title: "家里的服务器", value: view.details.server.text)
                }
                if let retrying = view.notice?.retrying {
                    RetryButton(retrying: retrying) { model.send(.retryConnection) }
                }
                WontonGroup {
                    NavigationLink(value: HomeRoute.connectionDetails) {
                        WontonRow(title: "详情", showsForward: true)
                    }
                }
                .buttonStyle(.plain)
                WontonGroup {
                    NavigationLink(value: HomeRoute.egress) {
                        WontonRow(title: "出网代理", showsForward: true)
                    }
                    WontonSeparator()
                    NavigationLink(value: HomeRoute.coreUpgrade) {
                        WontonRow(
                            title: "升级服务器",
                            value: plugins.viewModel.coreUpgrade.isPrompting ? "有新版本" : nil,
                            showsForward: true
                        )
                    }
                }
                .buttonStyle(.plain)
                WontonGroup {
                    NavigationLink(value: HomeRoute.invite) {
                        WontonRow(title: "添加设备", showsForward: true)
                    }
                    WontonSeparator()
                    NavigationLink(value: HomeRoute.pairing) {
                        WontonRow(title: "重新配对", showsForward: true)
                    }
                }
                .buttonStyle(.plain)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen("连接")
    }
}

struct ConnectionDetailsView: View {
    let model: HomeModel

    private var details: ConnectionDetails { model.viewModel.details }

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    if let path = model.viewModel.path {
                        WontonRow(title: "路径", value: path.kindText)
                        WontonSeparator()
                        WontonRow(title: "延迟", value: "\(path.rttMs) ms")
                        WontonSeparator()
                    }
                    WontonRow(title: "丢包", value: "\(details.lostPackets) / \(details.sentPackets)")
                    if let handshake = details.handshakeMs {
                        WontonSeparator()
                        WontonRow(title: "上次握手", value: "\(handshake) ms")
                    }
                    WontonSeparator()
                    WontonRow(title: "断开次数", value: "\(details.disconnects)")
                    if let connected = details.lastConnectedAtMs {
                        WontonSeparator()
                        WontonRow(title: "最后连上", value: Self.time(connected))
                    }
                }
                if let failure = details.lastFailure {
                    WontonGroup {
                        WontonRow(title: "上次失败", value: failure)
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen("详情")
    }

    private static func time(_ milliseconds: UInt64) -> String {
        Date(timeIntervalSince1970: Double(milliseconds) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}

private extension HomePath {
    var kindText: String {
        if lan { return "局域网" }
        return relay ? "中继" : "直连"
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
        case .ok: "正常"
        case .failing: "不通"
        case .unknown: "不确定"
        }
    }
}

/// 全屏的插件网页，带一个关闭按钮。没连上时先打开，显示“连接中…”，连上后自动加载；加载中途断开也一样，本地代理的错误页不露出来。
struct PluginPage: View {
    let title: String
    let url: URL
    let home: HomeViewModel
    var takeover: PageTakeover?
    let close: () -> Void
    @State private var started = false
    @State private var failures = 0
    @State private var reload = 0
    @Environment(\.wontonColors) private var colors

    /// 连上了还加载不了，试这么多次就照实说。
    private static let maxAttempts = 3

    private var isConnected: Bool {
        if case .connected = home.status { true } else { false }
    }

    private var gaveUp: Bool {
        isConnected && failures >= Self.maxAttempts
    }

    var body: some View {
        NavigationStack {
            ZStack {
                if started, let proxy = home.proxy {
                    PluginWebView(
                        url: url, proxy: proxy, reload: reload,
                        onLoaded: { failures = 0 },
                        onFailed: { failures += 1 }
                    )
                    .ignoresSafeArea(edges: .bottom)
                }
                if !started || failures > 0 { cover }
                if let takeover {
                    TakeoverNotice(takeover: takeover).frame(maxHeight: .infinity, alignment: .top)
                }
            }
            .navigationTitle(home.notice?.sentence ?? title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if let takeover {
                    ToolbarItem(placement: .topBarLeading) { TakeoverButton(takeover: takeover) }
                }
                ToolbarItem(placement: .topBarTrailing) { Button("关闭", action: close) }
            }
        }
        .onDisappear { takeover?.end() }
        .onChange(of: isConnected, initial: true) { _, connected in
            guard connected else { return }
            started = true
            if failures > 0 { reload += 1 }
        }
        .task(id: failures) {
            guard failures > 0, !gaveUp else { return }
            try? await Task.sleep(for: .seconds(2))
            if !Task.isCancelled, isConnected { reload += 1 }
        }
    }

    private var cover: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            if gaveUp {
                Text("网页打不开")
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                Button("重试") {
                    failures = 0
                    reload += 1
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            } else {
                ProgressView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(colors.canvas))
    }
}
