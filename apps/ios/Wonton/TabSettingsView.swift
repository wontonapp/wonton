import SwiftUI
import WontonClient

struct TabSettingsView: View {
    let plugins: PluginsModel
    @Environment(TabsModel.self) private var tabs
    @Environment(\.wontonColors) private var colors
    @State private var showsCandidates = false

    var body: some View {
        List {
            if let problem = tabs.viewModel.problem {
                ProblemNotice(problem: Problem(title: TabText.configurationFailed, detail: problem.message))
            }
            Section {
                Toggle(TabText.showLabels, isOn: Binding(
                    get: { tabs.viewModel.labels },
                    set: { tabs.send(.setTabLabels(visible: $0)) }
                ))
                .toggleStyle(WontonToggleStyle())
                .listRowBackground(Color(colors.canvas))
            }
            Section {
                ForEach(tabs.viewModel.items, id: \.tab) { item in
                    HStack(spacing: WontonTheme.spacing.s12) {
                        TabIconView(item: item, selected: false, plugins: plugins)
                        Text(item.title).wontonText(WontonTheme.typography.body)
                    }
                    .frame(minHeight: WontonTheme.size.target)
                    .deleteDisabled(!item.removable)
                    .listRowBackground(Color(colors.canvas))
                }
                .onMove(perform: move)
                .onDelete(perform: remove)
            }
            if !tabs.viewModel.candidates.isEmpty {
                Section {
                    Button { showsCandidates = true } label: {
                        WontonRow(title: TabText.addMore, showsForward: true)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(Color(colors.canvas))
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.editMode, .constant(.active))
        .navigationDestination(isPresented: $showsCandidates) {
            AddTabView(plugins: plugins).wontonPushedScreen(TabText.addToBar)
        }
        .environment(plugins)
    }

    private func remove(at offsets: IndexSet) {
        let items = offsets.map { tabs.viewModel.items[$0] }.filter(\.removable)
        for item in items { tabs.send(.unpinTab(tab: item.tab)) }
    }

    private func move(from source: IndexSet, to destination: Int) {
        var order = tabs.viewModel.items.map(\.tab)
        order.move(fromOffsets: source, toOffset: destination)
        for (index, tab) in order.enumerated() where tabs.viewModel.items[index].tab != tab {
            if !tabs.send(.moveTab(tab: tab, index: UInt32(index))) { return }
        }
    }
}

private struct AddTabView: View {
    let plugins: PluginsModel
    @Environment(TabsModel.self) private var tabs
    @Environment(\.wontonColors) private var colors
    @State private var pin: TabRef?

    var body: some View {
        List {
            ForEach(tabs.viewModel.candidates, id: \.tab) { candidate in
                Button { pin = candidate.tab } label: {
                    HStack(spacing: WontonTheme.spacing.s12) {
                        if let plugin = candidate.plugin {
                            PluginIconView(icon: plugin.icon, size: WontonTheme.size.icon, model: plugins)
                                .accessibilityHidden(true)
                        } else {
                            WontonIcon.settings.view().accessibilityHidden(true)
                        }
                        Text(title(candidate)).wontonText(WontonTheme.typography.body)
                        Spacer(minLength: 0)
                        WontonIcon.add.view().accessibilityHidden(true)
                    }
                    .frame(minHeight: WontonTheme.size.target)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .listRowBackground(Color(colors.canvas))
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.editMode, .constant(.inactive))
        .tabPinRequests($pin)
        .environment(plugins)
    }

    private func title(_ candidate: TabCandidate) -> String {
        guard let plugin = candidate.plugin else { return TabText.settings }
        guard let pageTitle = plugin.pageTitle else { return plugin.title }
        return "\(plugin.title) · \(pageTitle)"
    }
}

struct PluginTabPins: View {
    let plugin: InstalledPlugin
    @Environment(TabsModel.self) private var tabs
    @State private var pin: TabRef?

    var body: some View {
        if !plugin.tabChoices.isEmpty {
            DetailSection(title: TabText.pin) {
                ForEach(plugin.tabChoices) { choice in
                    Toggle(choice.title, isOn: Binding(
                        get: { tabs.isPinned(choice.tab) },
                        set: { enabled in
                            if enabled { pin = choice.tab } else { tabs.send(.unpinTab(tab: choice.tab)) }
                        }
                    ))
                    .toggleStyle(WontonToggleStyle())
                }
            }
            .tabPinRequests($pin)
        }
    }
}

struct PluginPinButton: View {
    let plugin: InstalledPlugin
    @State private var showsPages = false

    var body: some View {
        if !plugin.tabChoices.isEmpty {
            Button(TabText.pin) { showsPages = true }
                .buttonStyle(WontonSmallButtonStyle())
                .wontonSheet(isPresented: $showsPages, title: plugin.title) { _ in
                    PluginTabPins(plugin: plugin).padding(WontonTheme.spacing.pageInset)
                }
        }
    }
}

extension View {
    func tabPinRequests(_ request: Binding<TabRef?>) -> some View { modifier(TabPinRequests(request: request)) }
}

private struct TabPinRequests: ViewModifier {
    @Binding var request: TabRef?
    @Environment(TabsModel.self) private var tabs
    @Environment(PluginsModel.self) private var plugins

    func body(content: Content) -> some View {
        content
            .onChange(of: request) { _, tab in
                guard let tab, !tabs.viewModel.full else { return }
                tabs.send(.pinTab(tab: tab, replacing: nil))
                request = nil
            }
            .wontonSheet(isPresented: Binding(
                get: { request != nil && tabs.viewModel.full }, set: { if !$0 { request = nil } }
            ), title: TabText.replace) { close in
                VStack(spacing: 0) {
                    ForEach(tabs.viewModel.items.filter(\.removable), id: \.tab) { item in
                        Button {
                            guard let tab = request else { return }
                            close { tabs.send(.pinTab(tab: tab, replacing: item.tab)) }
                        } label: {
                            HStack(spacing: WontonTheme.spacing.s12) {
                                TabIconView(item: item, selected: false, plugins: plugins)
                                Text(item.title).wontonText(WontonTheme.typography.body)
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, WontonTheme.spacing.s16)
                            .padding(.vertical, WontonTheme.spacing.s12)
                            .frame(minHeight: WontonTheme.size.target)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
    }
}
