import SwiftUI
import WontonClient

/// “应用”：配对入口、已装插件和应用商店。只渲染核心给出的 ViewModel。
struct HomeView: View {
    @Bindable var model: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let settings: SettingsModel
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
    let showSettings: () -> Void
    let showServer: () -> Void
    var settingsPresented = false
    /// 开发时经 `WONTON_DEV_OPEN_WEB` 传入的插件网页，本地代理就绪后自动打开，真机上不用点按也能测。
    var devWebURL: URL?
    @Environment(\.wontonColors) private var colors
    @Environment(PagesModel.self) private var pages
    @State private var openedDevPage = false
    @Binding var path: NavigationPath
    @State private var confirmation: PluginConfirmation?
    @State private var uninstalling: UninstallTarget?
    @State private var menuPlugin: InstalledPlugin?

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if case .notConfigured = model.viewModel.status,
                   model.servers.adding || !model.servers.servers.contains(where: { $0.current }) {
                    PairingView(model: model)
                        .wontonRootScreen(String(localized: .Pairing.pairingTitle), hasActions: true) {
                            settingsLink
                        }
                } else {
                    launcher
                        .wontonRootScreen(String(localized: .Launcher.launcherTitle), hasActions: true) {
                            settingsLink
                        }
                }
            }
            .modifier(VisitingRoomModifier(model: chat, plugins: plugins, tab: .apps, path: $path))
            .navigationDestination(for: AppsRoute.self) { route in
                switch route {
                case .store: StoreView(model: plugins, open: open)
                }
            }
            .navigationDestination(for: PluginRoute.self) { route in
                PluginDetailView(model: plugins, chat: chat, route: route, open: open)
            }
            .onChange(of: plugins.viewModel.installed, initial: true) { _, installed in
                guard !openedDevPage, let devWebURL,
                      let plugin = installed?.plugins.first(where: { $0.webUrl == devWebURL.absoluteString }),
                      let choice = plugin.tabChoices.first else { return }
                openedDevPage = true
                pages.open(choice.tab)
            }
            .wontonDialog(problem: model.rejection, isPresented: isShowingRejection)
        }
        .wontonDialog(problem: plugins.rejection, isPresented: isShowingPluginRejection)
        .environment(\.goToInstalledApps, {
            pages.open(.apps)
        })
    }

    private var launcher: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s24) {
                if plugins.viewModel.actions.contains(.store) { storeEntry }
                if let installed = plugins.viewModel.installed {
                    installedPlugins(installed)
                    HomeStatusPanel(
                        home: model.viewModel, plugins: plugins.viewModel, cards: pendingCards,
                        showConnection: showServer, openCardRoom: openCardRoom
                    )
                } else if plugins.viewModel.actions.contains(.store) {
                    placeholder.frame(maxWidth: .infinity)
                }
                SharedAppsSection(model: plugins)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .refreshable {
            if plugins.viewModel.actions.contains(.store) { plugins.send(.refreshPlugins) }
            plugins.send(.refreshSharedApps(ownerUserId: nil))
        }
        .task {
            if plugins.viewModel.actions.contains(.store) { plugins.refresh(.refreshPlugins) }
            plugins.send(.refreshSharedApps(ownerUserId: nil))
        }
        .fullScreenCover(item: $confirmation) {
            InstallConfirmView(model: plugins, confirmation: $0, coreOffered: true)
        }
        // 核心取好预览、要主人拿主意的一键操作（ADR-0017）：商店列表和详情页都在这层栈里，确认页盖在最上面。
        // 确认后核心自己把这条撤走（排着队的放下一条进来），只有主人自己关才要告诉核心撤走。
        .onChange(of: plugins.viewModel.confirmation) { _, offered in
            confirmation = offered.map(PluginConfirmation.store)
        }
        .onChange(of: confirmation) { (old: PluginConfirmation?, new: PluginConfirmation?) in
            // 主人自己关掉确认页：核心撤走这条、放下一条。确认后核心已把这条撤走，ID 对不上，不会再进来。
            guard new == nil, let shown = old, case let .store(offer) = shown else { return }
            let current = plugins.viewModel.confirmation
            if current?.preview.id == offer.preview.id {
                plugins.send(.dismissStoreConfirmation)
            }
        }
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
            ProblemRow(problem: problem.problem) { plugins.send(.refreshPlugins) }
        }
        if let check = installed.upgradeCheck {
            ProblemRow(problem: check.problem) { plugins.send(.refreshPlugins) }
        }
        if installed.plugins.isEmpty {
            NoPlugins { path.append(AppsRoute.store) }
        } else {
            let cell = GridItem(.adaptive(minimum: WontonTheme.size.launcherCell), spacing: WontonTheme.spacing.s12)
            LazyVGrid(columns: [cell], spacing: WontonTheme.spacing.s24) {
                ForEach(installed.plugins, id: \.id) { plugin in
                    LauncherIcon(model: plugins, plugin: plugin) {
                        path.append(PluginRoute.installed(id: plugin.id))
                    }
                        .contentShape(Rectangle())
                        .onTapGesture { tap(plugin) }
                        .onLongPressGesture { menuPlugin = plugin }
                        .accessibilityElement(children: .contain)
                        .accessibilityAddTraits(.isButton)
                        .accessibilityAction { tap(plugin) }
                        .accessibilityAction(named: Text(.Launcher.launcherMenu)) { menuPlugin = plugin }
                }
            }
        }
    }

    private var storeEntry: some View {
        NavigationLink(value: AppsRoute.store) {
            HStack(spacing: WontonTheme.spacing.s12) {
                WontonIcon.store.view()
                    .foregroundStyle(Color(colors.accentInkOnFill))
                Text(.Store.storeTitle)
                    .wontonText(WontonTheme.typography.body)
                Spacer(minLength: 0)
                let updates = plugins.viewModel.updatable.count
                if updates > 0 {
                    Text(.Launcher.launcherUpdatesAvailable(count: updates))
                        .wontonText(WontonTheme.typography.supporting)
                        .multilineTextAlignment(.trailing)
                }
            }
            .foregroundStyle(Color(colors.textOnAccentFill))
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s12)
            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target)
            .accentFill(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
            .contentShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        }
        .buttonStyle(.plain)
    }

    /// 还没取到已安装的插件：取不到的原因和下一步，或者正在取。
    @ViewBuilder private var placeholder: some View {
        if let problem = plugins.viewModel.problem {
            ProblemView(problem: problem.problem) { plugins.send(.refreshPlugins) }
        } else if let rejection = plugins.refreshRejection {
            ProblemView(problem: rejection.problem) { plugins.send(.refreshPlugins) }
        } else {
            LoadingView()
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
            WontonSheetAction(title: String(localized: .Common.commonDetails), icon: .info) {
                path.append(PluginRoute.installed(id: plugin.id))
            }
        ]
        // 有 Bot 又有网页的 Agent 插件，点图标进私聊，网页从这里打开。
        if plugin.entries.count > 1, let web = plugin.entries.last {
            actions.append(WontonSheetAction(title: String(localized: .Launcher.launcherOpenWeb),
                icon: .web) { open(web) })
        }
        // 已加进“我的应用”的页给移出，没加的给加进。
        let mine = plugins.viewModel.sidebar.apps.compactMap(\.tab)
        for choice in plugin.tabChoices {
            let added = mine.contains(choice.tab)
            let title = (added ? TabText.removeFromMyApps : TabText.addToMyApps)
                + (plugin.tabChoices.count > 1 ? " · \(choice.title)" : "")
            actions.append(WontonSheetAction(title: title, icon: .apps) {
                if added {
                    pages.send(.removeMyApp(app: .page(tab: choice.tab)))
                } else {
                    pages.send(.addMyApp(app: .page(tab: choice.tab), title: nil, index: nil))
                }
            })
        }
        if let upgrade = plugin.upgrade {
            let title = String(localized: .Launcher.launcherUpgradeTo(upgrade.version))
            actions.append(WontonSheetAction(title: title, icon: .download) {
                plugins.send(.upgradeNow(pluginId: plugin.id))
            })
        }
        if plugin.removable {
            let title = String(localized: .Plugin3.pluginUninstallAction)
            actions.append(WontonSheetAction(title: title, icon: .trash, isDanger: true) {
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

    private var settingsLink: some View {
        WontonIconButton(icon: .settings, label: TabText.settings, action: showSettings)
            .overlay(alignment: .topTrailing) {
                if settings.hasServerUpgrade {
                    Circle().fill(Color(colors.primary)).frame(width: 12, height: 12)
                        .overlay(Circle().strokeBorder(Color(colors.canvas), lineWidth: 2))
                        .padding(WontonTheme.spacing.s8)
                        .allowsHitTesting(false)
                }
            }
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { !settingsPresented && model.rejection != nil },
            set: { isShowing in
                if !isShowing && !settingsPresented { model.rejection = nil }
            }
        )
    }

    private var isShowingPluginRejection: Binding<Bool> {
        Binding(
            get: { !settingsPresented && plugins.rejection != nil },
            set: { isShowing in
                if !isShowing && !settingsPresented { plugins.rejection = nil }
            }
        )
    }
}

enum AppsRoute: Hashable {
    case store
}

private struct NoPlugins: View {
    let showStore: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text(.Launcher.launcherEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.Launcher.launcherGoToStore, action: showStore)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
        }
        .frame(maxWidth: .infinity)
        .padding(.top, WontonTheme.spacing.s48)
    }
}

/// 主屏上的一个插件：圆角方形的图标和名字。能升级的、出了问题的在图标角上标出来，正在装、升级、卸载的转圈。
private struct LauncherIcon: View {
    let model: PluginsModel
    let plugin: InstalledPlugin
    let showDetails: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let size = WontonTheme.size.launcherIcon
        VStack(spacing: WontonTheme.spacing.s4) {
            PluginIconView(icon: plugin.icon, size: size, model: model)
                .overlay {
                    if plugin.isBusy {
                        RoundedRectangle(cornerRadius: size * WontonTheme.ratio.pluginIconRadius, style: .continuous)
                            .fill(.black.opacity(0.35))
                            .overlay { ProgressView().tint(.white) }
                    }
                }
                .overlay(alignment: .topTrailing) { badge.offset(x: 6, y: -6) }
                .overlay(alignment: .bottomTrailing) {
                    if !plugin.enabledRootPermissions.isEmpty {
                        // 点按区域 48pt，看得见的圆只有 28pt；整体往右下挪，圆心落在图标角上，不盖住图标本身。
                        Button(action: showDetails) {
                            WontonIcon.lock.view(size: WontonTheme.size.iconSmall)
                                .foregroundStyle(Color(colors.warning))
                                .padding(WontonTheme.spacing.s4)
                                .background(Circle().fill(Color(colors.canvas)))
                                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(String(localized: .Plugin1.pluginRootPermissionBadge))
                        .offset(x: 16, y: 16)
                    }
                }
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
                .accessibilityLabel(String(localized: .Launcher.launcherBadgeDegraded))
        } else if case .needsConfiguration = plugin.status {
            WontonIcon.wrenchFill.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.warning))
                .background(Circle().fill(Color(colors.canvas)))
                .accessibilityLabel(String(localized: .Launcher.launcherBadgeNeedsConfiguration))
        } else if plugin.upgrade != nil {
            Circle().fill(Color(colors.primary)).frame(width: 12, height: 12)
                .overlay(Circle().strokeBorder(Color(colors.canvas), lineWidth: 2))
                .accessibilityLabel(String(localized: .Sidebar.sidebarUpdatable))
        }
    }
}

extension CoreUpgradeViewModel {
    /// 有新版本，主人没跳过，也没在升级。
    var isPrompting: Bool { offer != nil && !offerSkipped }
}
