import SwiftUI
import WontonClient

/// 设置里的“我的应用”：放哪些、顺序、新装应用自动加入。数据是核心算好的（侧边栏同一份）。
struct MyAppsSettingsView: View {
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins
    @Environment(\.wontonColors) private var colors
    @Environment(\.editMode) private var editMode
    @State private var showsCandidates = false

    var body: some View {
        List {
            Section {
                Toggle(String(localized: .myappAutoAdd), isOn: Binding(
                    get: { pages.viewModel.autoAdd },
                    set: { pages.send(.setAutoAddApps(enabled: $0)) }
                ))
            }
            Section(String(localized: .sidebarMyApps)) {
                let apps = plugins.viewModel.sidebar.apps
                ForEach(apps, id: \.self) { app in
                    if let appRef = app.myAppRef {
                        HStack(spacing: WontonTheme.spacing.s12) {
                            if app.pluginId != nil {
                                PluginIconView(icon: app.icon, size: WontonTheme.size.icon, model: plugins)
                            } else {
                                WontonIcon.settings.view(size: WontonTheme.size.icon)
                                    .foregroundStyle(Color(colors.textPrimary))
                            }
                            Text(app.title)
                        }
                    }
                }
                .onMove { source, destination in
                    for sourceIndex in source {
                        guard let app = apps[safe: sourceIndex], let appRef = app.myAppRef else { continue }
                        let target = destinationIndex(destination, removingAt: sourceIndex, count: apps.count)
                        pages.send(.moveMyApp(app: appRef, index: UInt32(target)))
                    }
                }
                .onDelete { source in
                    for index in source {
                        guard let app = apps[safe: index], let appRef = app.myAppRef else { continue }
                        pages.send(.removeMyApp(app: appRef))
                    }
                }
                if !pages.viewModel.myAppCandidates.isEmpty, editMode?.wrappedValue != .active {
                    Button(String(localized: .myappAddMore)) { showsCandidates = true }
                }
            }
            if let problem = pages.viewModel.problem {
                Section {
                    ProblemNotice(problem: Problem(
                        title: String(localized: .tabSettingsUnreadable),
                        detail: problem.message
                    ))
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
                MyAppCandidates().wontonModalScreen(TabText.addToMyApps)
            }
        }
    }

    /// onMove 的 destination 是“插入位置”（删掉拖动行之前算的），换算成核心要的下标。
    private func destinationIndex(_ destination: Int, removingAt source: Int, count: Int) -> Int {
        var target = destination
        if destination > source { target -= 1 }
        return max(0, min(count - 1, target))
    }
}

/// 还没加进“我的应用”的页面，逐行加进去。
private struct MyAppCandidates: View {
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins
    @Environment(\.wontonColors) private var colors

    var body: some View {
        List {
            ForEach(pages.viewModel.myAppCandidates, id: \.tab) { (candidate: TabCandidate) in
                Button {
                    pages.send(.addMyApp(app: .page(tab: candidate.tab), title: nil, index: nil))
                } label: {
                    HStack(spacing: WontonTheme.spacing.s12) {
                        if let plugin = candidate.plugin {
                            PluginIconView(icon: plugin.icon, size: WontonTheme.size.icon, model: plugins)
                            Text(plugin.pageTitle.map { "\(plugin.title) · \($0)" } ?? plugin.title)
                        } else {
                            Text(String(localized: .tabSettings))
                        }
                        Spacer(minLength: 0)
                        WontonIcon.add.view(size: WontonTheme.size.icon)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                }
                .foregroundStyle(.primary)
            }
            if let problem = pages.viewModel.problem {
                Section {
                    ProblemNotice(problem: Problem(
                        title: String(localized: .tabSettingsUnreadable),
                        detail: problem.message
                    ))
                }
            }
        }
        .scrollContentBackground(.hidden)
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
