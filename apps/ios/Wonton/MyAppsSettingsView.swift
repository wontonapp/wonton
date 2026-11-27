import SwiftUI
import WontonClient

/// 设置里的“我的应用”：放哪些、顺序、新装应用自动加入。数据是核心算好的（侧边栏同一份）。
/// 应用行点开是插件详情页，分享给家人的入口和已有分享都在那里（#693：设置里找不到授权入口）。
/// 详情里的“打开”先收起设置浮层再走主界面（MainView 注入的 [open]），“查看已安装”同（[checkInstalled]）。
struct MyAppsSettingsView: View {
    let chat: ChatModel
    let open: (PluginEntry) -> Void
    let checkInstalled: () -> Void
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins
    @Environment(\.wontonColors) private var colors
    @Environment(\.editMode) private var editMode
    @State private var showsCandidates = false
    @State private var openedPluginId: String?

    var body: some View {
        List {
            Section {
                Toggle(String(localized: .myappAutoAdd), isOn: Binding(
                    get: { pages.viewModel.autoAdd },
                    set: { pages.send(.pages(action: .setAutoAddApps(enabled: $0))) }
                ))
            }
            Section(String(localized: .Sidebar.sidebarMyApps)) {
                let apps = plugins.viewModel.sidebar.apps
                ForEach(apps, id: \.self) { app in
                    if let appRef = app.myAppRef {
                        if let pluginId = app.pluginId {
                            Button { openedPluginId = pluginId } label: { appRow(app) }
                                .foregroundStyle(.primary)
                        } else {
                            appRow(app)
                        }
                    }
                }
                .onMove { source, destination in
                    for sourceIndex in source {
                        guard let app = apps[safe: sourceIndex], let appRef = app.myAppRef else { continue }
                        let target = destinationIndex(destination, removingAt: sourceIndex, count: apps.count)
                        pages.send(.pages(action: .moveMyApp(app: appRef, index: UInt32(target))))
                    }
                }
                .onDelete { source in
                    for index in source {
                        guard let app = apps[safe: index], let appRef = app.myAppRef else { continue }
                        pages.send(.pages(action: .removeMyApp(app: appRef)))
                    }
                }
                if editMode?.wrappedValue != .active {
                    Button(String(localized: .myappAddMore)) { showsCandidates = true }
                }
            }
            if let problem = pages.viewModel.problem {
                Section {
                    ProblemNotice(problem: problem.problem)
                }
            }
        }
        .scrollContentBackground(.hidden)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                EditButton()
            }
        }
        .sheet(isPresented: $showsCandidates) {
            NavigationStack {
                MyAppCandidates().wontonModalScreen(TabText.addToMyApps, usesSystemToolbar: true)
            }
        }
        .sheet(isPresented: detailShown) {
            if let pluginId = openedPluginId {
                NavigationStack {
                    PluginDetailView(model: plugins, chat: chat, route: .installed(id: pluginId)) { entry in
                        openedPluginId = nil
                        open(entry)
                    }
                }
                .environment(\.goToInstalledApps, checkInstalled)
            }
        }
    }

    /// 应用行：插件行带一句“分享给家人”和前进箭头，点开是详情页上的分享区。
    @ViewBuilder private func appRow(_ app: SidebarApp) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            if app.pluginId != nil {
                PluginIconView(icon: app.icon, size: WontonTheme.size.icon, model: plugins)
            } else {
                WontonIcon.settings.view(size: WontonTheme.size.icon)
                    .foregroundStyle(Color(colors.textPrimary))
            }
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(app.title)
                if app.pluginId != nil {
                    Text(.Share0.shareAppsRow)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            Spacer(minLength: 0)
            if app.pluginId != nil {
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
    }

    private var detailShown: Binding<Bool> {
        Binding(
            get: { openedPluginId != nil },
            set: { if !$0 { openedPluginId = nil } }
        )
    }

    /// onMove 的 destination 是“插入位置”（删掉拖动行之前算的），换算成核心要的下标。
    private func destinationIndex(_ destination: Int, removingAt source: Int, count: Int) -> Int {
        var target = destination
        if destination > source { target -= 1 }
        return max(0, min(count - 1, target))
    }
}

/// 还没加进“我的应用”的页面，逐行加进去；最后一行填网址加公网网页或插件里的某一页。
private struct MyAppCandidates: View {
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins
    @Environment(\.wontonColors) private var colors
    @State private var url = ""

    var body: some View {
        List {
            ForEach(pages.viewModel.myAppCandidates, id: \.tab) { (candidate: TabCandidate) in
                Button {
                    pages.send(.pages(action: .addMyApp(app: .page(tab: candidate.tab), title: nil, index: nil)))
                } label: {
                    HStack(spacing: WontonTheme.spacing.s12) {
                        if let plugin = candidate.plugin {
                            PluginIconView(icon: plugin.icon, size: WontonTheme.size.icon, model: plugins)
                            Text(plugin.pageTitle.map { "\(plugin.title) · \($0)" } ?? plugin.title)
                        } else {
                            Text(String(localized: .Tab2.tabSettings))
                        }
                        Spacer(minLength: 0)
                        WontonIcon.add.view(size: WontonTheme.size.icon)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                }
                .foregroundStyle(.primary)
            }
            Section {
                HStack(spacing: WontonTheme.spacing.s8) {
                    TextField("https://", text: $url)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                        .onSubmit(addURL)
                    Button(String(localized: .myappAddUrl), action: addURL)
                        .buttonStyle(WontonSmallButtonStyle())
                        .disabled(url.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            if let problem = pages.viewModel.problem {
                Section {
                    ProblemNotice(problem: problem.problem)
                }
            }
        }
        .scrollContentBackground(.hidden)
    }

    /// 网址对不对由核心判断（只收 https、Bot 的网页不收），不对的原因出在下面的问题里。
    private func addURL() {
        let value = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        pages.send(.pages(action: .addMyAppUrl(url: value, title: nil)))
        url = ""
    }
}

extension SidebarApp {
    /// 这一行对应的存储引用：页面行按页面，插件行按插件（Bot 私聊行）。
    var myAppRef: MyAppRef? {
        if let tab { return .page(tab: tab) }
        if let pluginId { return .plugin(pluginId: pluginId) }
        return nil
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
