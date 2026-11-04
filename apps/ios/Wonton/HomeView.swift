import SwiftUI
import WontonClient

/// “应用”：还没配对时是配对页；配上以后按图标和名字列出已装的插件，最后一格进应用商店，连接和设备在右上角。只渲染
/// 核心给出的 ViewModel。
struct HomeView: View {
    @Bindable var model: HomeModel
    let plugins: PluginsModel
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
    /// 开发时经 `WONTON_DEV_OPEN_WEB` 传入的插件网页，本地代理就绪后自动打开，真机上不用点按也能测。
    var devWebURL: URL?
    @State private var path = NavigationPath()
    @State private var isShowingPage = false
    @State private var confirmation: PluginConfirmation?
    @State private var uninstalling: UninstallTarget?
    @State private var menuPlugin: InstalledPlugin?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if case .notConfigured = model.viewModel.status {
                    PairingView(model: model)
                        .wontonRootScreen("配对") { EmptyView() }
                } else {
                    launcher
                        .wontonRootScreen("应用") {
                            connectionLink
                        }
                }
            }
            .navigationDestination(for: HomeRoute.self) { route in
                switch route {
                case .connection:
                    ConnectionView(model: model)
                case .invite:
                    InviteView(model: model)
                case .pairing:
                    PairingView(model: model) { path.removeLast() }
                        .wontonPushedScreen("重新配对")
                case .egress:
                    EgressSettingsView(model: plugins)
                case .store:
                    StoreView(model: plugins, open: open)
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
            .wontonDialog("操作被拒绝", message: model.rejection, isPresented: isShowingRejection)
        }
        .wontonDialog("操作被拒绝", message: plugins.rejection, isPresented: isShowingPluginRejection)
    }

    private var launcher: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s24) {
                if let installed = plugins.viewModel.installed {
                    installedPlugins(installed)
                    HomeStatusPanel(
                        home: model.viewModel, plugins: plugins.viewModel, cards: pendingCards,
                        showConnection: { path.append(HomeRoute.connection) }, openCardRoom: openCardRoom
                    )
                }
            }
            .padding(WontonTheme.spacing.pageInset)
            // 内容为空时 ScrollView 的宽度随内容缩成零，盖在上面的占位也跟着变窄。
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .overlay {
            if plugins.viewModel.installed == nil { placeholder }
        }
        .refreshable { plugins.send(.refreshPlugins) }
        .task { plugins.refresh(.refreshPlugins) }
        .fullScreenCover(item: $confirmation) { InstallConfirmView(model: plugins, confirmation: $0) }
        .uninstallDialog($uninstalling, model: plugins)
        .wontonActionSheet(
            isPresented: isShowingMenu,
            title: menuPlugin?.title,
            actions: menuPlugin.map(menu(for:)) ?? []
        )
        .sensoryFeedback(.impact, trigger: menuPlugin?.id) { _, id in id != nil }
    }

    @ViewBuilder private func installedPlugins(_ installed: InstalledPlugins) -> some View {
        if let problem = plugins.viewModel.problem {
            ProblemRow(message: problem.message, nextStep: problem.reason.nextStep)
        }
        if let check = installed.upgradeCheck {
            ProblemRow(message: check.message, nextStep: nil)
        }
        if installed.plugins.isEmpty {
            NoPlugins { path.append(HomeRoute.store) }
        } else {
            let cell = GridItem(.adaptive(minimum: WontonTheme.size.launcherCell), spacing: WontonTheme.spacing.s12)
            LazyVGrid(columns: [cell], spacing: WontonTheme.spacing.s24) {
                ForEach(installed.plugins, id: \.id) { plugin in
                    LauncherIcon(model: plugins, plugin: plugin)
                        .contentShape(Rectangle())
                        .onTapGesture { tap(plugin) }
                        .onLongPressGesture { menuPlugin = plugin }
                        .accessibilityElement(children: .combine)
                        .accessibilityAddTraits(.isButton)
                        .accessibilityAction { tap(plugin) }
                        .accessibilityAction(named: "菜单") { menuPlugin = plugin }
                }
                NavigationLink(value: HomeRoute.store) { StoreTile() }
                    .buttonStyle(.plain)
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

    private func menu(for plugin: InstalledPlugin) -> [WontonSheetAction] {
        var actions = [
            WontonSheetAction(title: "详情", icon: .info) { path.append(PluginRoute.installed(id: plugin.id)) }
        ]
        // 有 Bot 又有网页的 Agent 插件，点图标进私聊，网页从这里打开。
        if plugin.entries.count > 1, let web = plugin.entries.last {
            actions.append(WontonSheetAction(title: "打开网页", icon: .web) { open(web) })
        }
        if let upgrade = plugin.upgrade {
            actions.append(WontonSheetAction(title: "升级到 \(upgrade.version)", icon: .download) {
                confirmation = .upgrade(id: plugin.id)
            })
        }
        if plugin.removable {
            actions.append(WontonSheetAction(title: "卸载", icon: .trash, isDanger: true) {
                uninstalling = UninstallTarget(id: plugin.id, title: plugin.title)
            })
        }
        return actions
    }

    private var isShowingMenu: Binding<Bool> {
        Binding(
            get: { menuPlugin != nil },
            set: { isShowing in
                if !isShowing { menuPlugin = nil }
            }
        )
    }

    /// 连不上时换成警告；正在连时变浅。
    @ViewBuilder private var connectionLink: some View {
        switch model.viewModel.status {
        case .disconnected:
            WontonIconLink(icon: .warning, label: "连接", value: HomeRoute.connection, tint: colors.danger)
        case .connecting:
            WontonIconLink(icon: .connection, label: "连接", value: HomeRoute.connection, tint: colors.textSecondary)
        case .connected, .notConfigured:
            WontonIconLink(icon: .connection, label: "连接", value: HomeRoute.connection)
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
    case invite
    case pairing
    case egress
    case store
}

private struct NoPlugins: View {
    let showStore: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text("还没有装应用")
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button("去应用商店", action: showStore)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
        }
        .frame(maxWidth: .infinity)
        .padding(.top, WontonTheme.spacing.s48)
    }
}

/// 图标网格的最后一格。
private struct StoreTile: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let size = WontonTheme.size.launcherIcon
        VStack(spacing: WontonTheme.spacing.s4) {
            RoundedRectangle(cornerRadius: size * WontonTheme.ratio.pluginIconRadius, style: .continuous)
                .fill(Color(colors.surfaceSubtle))
                .frame(width: size, height: size)
                .overlay { WontonIcon.store.view(size: WontonTheme.size.icon) }
                .foregroundStyle(Color(colors.textPrimary))
            Text("应用商店")
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
    }
}

/// 主屏上的一个插件：圆角方形的图标和名字。能升级的、出了问题的在图标角上标出来，正在装、升级、卸载的转圈。
private struct LauncherIcon: View {
    let model: PluginsModel
    let plugin: InstalledPlugin
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let size = WontonTheme.size.launcherIcon
        VStack(spacing: WontonTheme.spacing.s4) {
            PluginIconView(icon: plugin.icon, category: plugin.category, size: size, model: model)
                .overlay {
                    if plugin.isBusy {
                        RoundedRectangle(cornerRadius: size * WontonTheme.ratio.pluginIconRadius, style: .continuous)
                            .fill(.black.opacity(0.35))
                            .overlay { ProgressView().tint(.white) }
                    }
                }
                .overlay(alignment: .topTrailing) { badge.offset(x: 6, y: -6) }
            Text(plugin.title)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
    }

    @ViewBuilder private var badge: some View {
        if case .degraded = plugin.status {
            WontonIcon.warningCircleFill.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.danger))
                .background(Circle().fill(Color(colors.canvas)))
        } else if plugin.upgrade != nil {
            Circle().fill(Color(colors.primary)).frame(width: 12, height: 12)
                .overlay(Circle().strokeBorder(Color(colors.canvas), lineWidth: 2))
        }
    }
}

/// 和家里服务器的连接，以及添加设备、重新配对。
private struct ConnectionView: View {
    let model: HomeModel

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    WontonRow(title: "状态", value: model.viewModel.status.text)
                    if let path = model.viewModel.path {
                        WontonSeparator()
                        WontonRow(title: "路径", value: "\(path.relay ? "中继" : "直连") · \(path.rttMs) ms")
                    }
                }
                WontonGroup {
                    NavigationLink(value: HomeRoute.egress) {
                        WontonRow(title: "出网代理", showsForward: true)
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
