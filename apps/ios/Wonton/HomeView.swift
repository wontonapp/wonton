import SwiftUI
import WontonClient

/// “家”：还没配对时是配对页；配上以后是主屏，按图标和名字列出已装的插件，连接和设备在右上角。只渲染核心给出的
/// ViewModel。
struct HomeView: View {
    @Bindable var model: HomeModel
    let plugins: PluginsModel
    let appearance: AppearanceModel
    let open: (PluginEntry) -> Void
    let showStore: () -> Void
    /// 开发时经 `WONTON_DEV_OPEN_WEB` 传入的插件网页，本地代理就绪后自动打开，真机上不用点按也能测。
    var devWebURL: URL?
    @State private var path = NavigationPath()
    @State private var isShowingPage = false
    @State private var isPairing = false
    @State private var isInviting = false
    @State private var confirmation: PluginConfirmation?
    @State private var uninstalling: UninstallTarget?

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if case .notConfigured = model.viewModel.status {
                    PairingView(model: model)
                } else {
                    launcher
                }
            }
            .navigationTitle("家")
            .toolbar {
                // 外观不依赖家里的服务器，没配对时也能进。
                ToolbarItem(placement: .topBarTrailing) {
                    NavigationLink("外观", value: HomeRoute.appearance)
                }
                if model.viewModel.status != .notConfigured {
                    ToolbarItem(placement: .topBarTrailing) {
                        NavigationLink(value: HomeRoute.connection) { connectionLabel }
                    }
                }
            }
            .navigationDestination(for: HomeRoute.self) { route in
                switch route {
                case .connection:
                    ConnectionView(model: model, isPairing: $isPairing, isInviting: $isInviting)
                case .appearance:
                    AppearanceView(model: appearance)
                }
            }
            .navigationDestination(for: PluginRoute.self) { PluginDetailView(model: plugins, route: $0, open: open) }
            .onChange(of: model.viewModel.proxy, initial: true) { _, proxy in
                if devWebURL != nil, proxy != nil { isShowingPage = true }
            }
            .fullScreenCover(isPresented: $isShowingPage) {
                if let url = devWebURL, let proxy = model.viewModel.proxy {
                    PluginPage(title: url.host() ?? "", url: url, proxy: proxy) { isShowingPage = false }
                }
            }
            .sheet(isPresented: $isPairing) {
                NavigationStack {
                    PairingView(model: model) { isPairing = false }
                        .navigationTitle("重新配对")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("关闭") { isPairing = false }
                            }
                        }
                }
            }
            .sheet(isPresented: $isInviting) {
                InviteView(model: model) { isInviting = false }
            }
            .alert("操作被拒绝", isPresented: isShowingRejection, presenting: model.rejection) { _ in
                Button("好") {}
            } message: { reason in
                Text(reason)
            }
        }
    }

    private var launcher: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let installed = plugins.viewModel.installed {
                    installedPlugins(installed)
                }
            }
            .padding()
            // 内容为空时 ScrollView 的宽度随内容缩成零，盖在上面的占位也跟着变窄。
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .overlay {
            if plugins.viewModel.installed == nil { placeholder }
        }
        .refreshable { plugins.send(.refreshPlugins) }
        .task { plugins.refresh(.refreshPlugins) }
        .sheet(item: $confirmation) { InstallConfirmView(model: plugins, confirmation: $0) }
        .uninstallDialog($uninstalling, model: plugins)
        .alert("操作被拒绝", isPresented: isShowingPluginRejection, presenting: plugins.rejection) { _ in
            Button("好") {}
        } message: { reason in
            Text(reason)
        }
    }

    @ViewBuilder private func installedPlugins(_ installed: InstalledPlugins) -> some View {
        if let problem = plugins.viewModel.problem {
            ProblemRow(message: problem.message, nextStep: problem.reason.nextStep)
        }
        if let check = installed.upgradeCheck {
            ProblemRow(message: check.message, nextStep: nil)
        }
        if installed.plugins.isEmpty {
            ContentUnavailableView {
                Label("还没有装插件", systemImage: "square.grid.2x2")
            } actions: {
                Button("去商店", action: showStore).buttonStyle(.bordered)
            }
        } else {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 76), spacing: 12)], spacing: 22) {
                ForEach(installed.plugins, id: \.id) { plugin in
                    Button { tap(plugin) } label: { LauncherIcon(model: plugins, plugin: plugin) }
                        .buttonStyle(.plain)
                        .contextMenu { menu(for: plugin) }
                }
            }
        }
    }

    /// 还没取到已安装的插件：取不到的原因和下一步，或者正在取。
    @ViewBuilder private var placeholder: some View {
        if let problem = plugins.viewModel.problem {
            ProblemView(message: problem.message, nextStep: problem.reason.nextStep) {
                plugins.send(.refreshPlugins)
            }
        } else if let rejection = plugins.refreshRejection {
            ProblemView(message: rejection, nextStep: nil) { plugins.send(.refreshPlugins) }
        } else {
            ProgressView()
        }
    }

    /// 点图标：Agent 插件进和它的 Bot 的私聊，有网页的打开小程序；都没有、或者正在装、升级、卸载时看详情。
    private func tap(_ plugin: InstalledPlugin) {
        if !plugin.isBusy, let entry = plugin.entries.first {
            open(entry)
        } else {
            path.append(PluginRoute.installed(id: plugin.id))
        }
    }

    @ViewBuilder private func menu(for plugin: InstalledPlugin) -> some View {
        Button("详情", systemImage: "info.circle") { path.append(PluginRoute.installed(id: plugin.id)) }
        // 有 Bot 又有网页的 Agent 插件，点图标进私聊，网页从这里打开。
        if plugin.entries.count > 1, let web = plugin.entries.last {
            Button("打开网页", systemImage: "safari") { open(web) }
        }
        if let upgrade = plugin.upgrade {
            Button("升级到 \(upgrade.version)", systemImage: "arrow.down.circle") {
                confirmation = .upgrade(id: plugin.id)
            }
        }
        if plugin.removable {
            Button("卸载", systemImage: "trash", role: .destructive) {
                uninstalling = UninstallTarget(id: plugin.id, title: plugin.title)
            }
        }
    }

    @ViewBuilder private var connectionLabel: some View {
        switch model.viewModel.status {
        case .disconnected:
            Label("连接", systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
        case .connecting:
            Label("连接", systemImage: "antenna.radiowaves.left.and.right").foregroundStyle(.secondary)
        case .connected, .notConfigured:
            Label("连接", systemImage: "antenna.radiowaves.left.and.right")
        }
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { model.rejection != nil },
            set: { isShowing in
                if !isShowing { model.rejection = nil }
            }
        )
    }

    private var isShowingPluginRejection: Binding<Bool> {
        Binding(
            get: { plugins.rejection != nil },
            set: { isShowing in
                if !isShowing { plugins.rejection = nil }
            }
        )
    }
}

private enum HomeRoute: Hashable {
    case connection
    case appearance
}

/// 主屏上的一个插件：圆角方形的图标和名字。能升级的、出了问题的在图标角上标出来，正在装、升级、卸载的转圈。
private struct LauncherIcon: View {
    let model: PluginsModel
    let plugin: InstalledPlugin

    var body: some View {
        VStack(spacing: 6) {
            PluginIconView(icon: plugin.icon, kind: plugin.kind, size: 62, model: model)
                .overlay {
                    if plugin.isBusy {
                        RoundedRectangle(cornerRadius: 62 * 0.2237, style: .continuous)
                            .fill(.black.opacity(0.35))
                            .overlay { ProgressView().tint(.white) }
                    }
                }
                .overlay(alignment: .topTrailing) { badge.offset(x: 6, y: -6) }
            Text(plugin.title).font(.caption).lineLimit(1)
        }
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
    }

    @ViewBuilder private var badge: some View {
        if case .degraded = plugin.status {
            Image(systemName: "exclamationmark.circle.fill")
                .symbolRenderingMode(.multicolor)
                .font(.title3)
                .background(Circle().fill(.background))
        } else if plugin.upgrade != nil {
            Circle().fill(Color.accentColor).frame(width: 12, height: 12)
                .overlay(Circle().strokeBorder(.background, lineWidth: 2))
        }
    }
}

/// 和家里服务器的连接，以及添加设备、重新配对。
private struct ConnectionView: View {
    let model: HomeModel
    @Binding var isPairing: Bool
    @Binding var isInviting: Bool

    var body: some View {
        Form {
            Section {
                LabeledContent("状态", value: model.viewModel.status.text)
                if let path = model.viewModel.path {
                    LabeledContent("路径", value: "\(path.relay ? "中继" : "直连") · \(path.rttMs) ms")
                }
            }
            Section {
                Button("添加设备") { isInviting = true }
                Button("重新配对") { isPairing = true }
            }
        }
        .navigationTitle("连接")
    }
}

extension HomeStatus {
    var text: String {
        switch self {
        case .notConfigured:
            return "还没有配对"
        case .connecting:
            return "连接中"
        case .connected:
            return "已连接"
        case let .disconnected(reason, message):
            switch reason {
            case .unauthorized:
                return "家里的服务器不认这台设备，请重新扫码配对"
            case .transportUnavailable, .upstreamUnavailable, .protocol:
                return message
            }
        }
    }
}

/// 全屏的插件网页（小程序），带一个关闭按钮。
struct PluginPage: View {
    let title: String
    let url: URL
    let proxy: ProxySettings
    let close: () -> Void

    var body: some View {
        NavigationStack {
            PluginWebView(url: url, proxy: proxy)
                .ignoresSafeArea(edges: .bottom)
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    Button("关闭", action: close)
                }
        }
    }
}
