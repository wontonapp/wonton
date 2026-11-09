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
    @Environment(TabsModel.self) private var tabs
    @State private var pin: TabRef?
    @State private var openedDevPage = false
    @State private var path = NavigationPath()
    @State private var confirmation: PluginConfirmation?
    @State private var uninstalling: UninstallTarget?
    @State private var menuPlugin: InstalledPlugin?

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if case .notConfigured = model.viewModel.status {
                    PairingView(model: model)
                        .wontonRootScreen(String(localized: .pairingTitle), hasActions: !tabs.isPinned(.settings)) {
                            if !tabs.isPinned(.settings) {
                                settingsLink
                            }
                        }
                } else {
                    launcher
                        .wontonRootScreen(String(localized: .launcherTitle), hasActions: !tabs.isPinned(.settings)) {
                            if !tabs.isPinned(.settings) {
                                settingsLink
                            }
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
                tabs.select(choice.tab)
            }
            .wontonDialog(problem: model.rejection, isPresented: isShowingRejection)
        }
        .wontonDialog(problem: plugins.rejection, isPresented: isShowingPluginRejection)
        .tabPinRequests($pin)
    }

    private var launcher: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s24) {
                storeEntry
                if let installed = plugins.viewModel.installed {
                    installedPlugins(installed)
                    HomeStatusPanel(
                        home: model.viewModel, plugins: plugins.viewModel, cards: pendingCards,
                        showConnection: showServer, openCardRoom: openCardRoom
                    )
                } else {
                    placeholder.frame(maxWidth: .infinity)
                }
            }
            .padding(WontonTheme.spacing.pageInset)
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
                        .accessibilityAction(named: Text(.launcherMenu)) { menuPlugin = plugin }
                }
            }
        }
    }

    private var storeEntry: some View {
        NavigationLink(value: AppsRoute.store) {
            HStack(spacing: WontonTheme.spacing.s12) {
                WontonIcon.store.view()
                    .foregroundStyle(Color(colors.accentInkOnFill))
                Text(.storeTitle)
                    .wontonText(WontonTheme.typography.body)
                Spacer(minLength: 0)
                if let installed = plugins.viewModel.installed {
                    let updates = installed.plugins.reduce(0) { $0 + ($1.upgrade != nil ? 1 : 0) }
                    if updates > 0 {
                        Text(.launcherUpdatesAvailable(count: updates))
                            .wontonText(WontonTheme.typography.supporting)
                            .multilineTextAlignment(.trailing)
                    }
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
            WontonSheetAction(title: String(localized: .commonDetails), icon: .info) {
                path.append(PluginRoute.installed(id: plugin.id))
            }
        ]
        // 有 Bot 又有网页的 Agent 插件，点图标进私聊，网页从这里打开。
        if plugin.entries.count > 1, let web = plugin.entries.last {
            actions.append(WontonSheetAction(title: String(localized: .launcherOpenWeb), icon: .web) { open(web) })
        }
        for choice in plugin.tabChoices {
            let pinned = tabs.isPinned(choice.tab)
            let title = (pinned ? TabText.unpin : TabText.pin)
                + (plugin.tabChoices.count > 1 ? " · \(choice.title)" : "")
            actions.append(WontonSheetAction(title: title, icon: .apps) {
                if pinned { tabs.send(.unpinTab(tab: choice.tab)) } else { pin = choice.tab }
            })
        }
        if let upgrade = plugin.upgrade {
            let title = String(localized: .launcherUpgradeTo(upgrade.version))
            actions.append(WontonSheetAction(title: title, icon: .download) {
                confirmation = .upgrade(id: plugin.id)
            })
        }
        if plugin.removable {
            let title = String(localized: .pluginUninstallAction)
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
            get: { tabs.viewModel.selected == .apps && !settingsPresented && model.rejection != nil },
            set: { isShowing in
                if !isShowing && tabs.viewModel.selected == .apps && !settingsPresented { model.rejection = nil }
            }
        )
    }

    private var isShowingPluginRejection: Binding<Bool> {
        Binding(
            get: { tabs.viewModel.selected == .apps && !settingsPresented && plugins.rejection != nil },
            set: { isShowing in
                if !isShowing && tabs.viewModel.selected == .apps && !settingsPresented { plugins.rejection = nil }
            }
        )
    }
}

private enum AppsRoute: Hashable {
    case store
}

private struct NoPlugins: View {
    let showStore: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Text(.launcherEmpty)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Button(.launcherGoToStore, action: showStore)
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
                        .accessibilityLabel(String(localized: .pluginRootPermissionBadge))
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
        } else if plugin.upgrade != nil {
            Circle().fill(Color(colors.primary)).frame(width: 12, height: 12)
                .overlay(Circle().strokeBorder(Color(colors.canvas), lineWidth: 2))
        }
    }
}

extension CoreUpgradeViewModel {
    /// 有新版本，主人没跳过，也没在升级。
    var isPrompting: Bool { offer != nil && !offerSkipped }
}
